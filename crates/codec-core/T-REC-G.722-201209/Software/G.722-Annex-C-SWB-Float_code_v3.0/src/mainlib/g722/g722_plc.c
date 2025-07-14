/* ITU G.722 3rd Edition (2012-09) */

/*--------------------------------------------------------------------------
 ITU-T G.722 Annex C (ex G.722-SWB-Float) Source Code
 Software Release 1.01 (2012-07)
 (C) 2012 France Telecom, Huawei Technologies, VoiceAge Corp., NTT.
--------------------------------------------------------------------------*/
#include <math.h>

#include "floatutil.h"
#include "pcmswb_common.h"
#include "g722.h"
#include "g722_plc.h"
#include "lpctool.h"

/**********************************
* declaration of PLC subroutines *
**********************************/

/* lower-band analysis (main subroutine: G722PLC_ana) */
static void  G722PLC_ana_flt(G722PLC_STATE_FLT * plc_state, g722_state *decoder);
static Short  G722PLC_pitch_ol_flt(Float * signal, Float *maxco);
static Short  G722PLC_classif_modif_flt(Float maxco, Short nbl, Short nbh, Float* mem_speech, int l_mem_speech,
                                     Float* mem_exc, Short* t0
                                     );
static void    G722PLC_autocorr_flt(Float * x, Float * R, Short ord, Short len);
static void    G722PLC_lpc_flt(G722PLC_STATE_FLT * plc_state, Float * mem_speech); /* interface modified for ONLY_LTP_DC_REMOVE */
static void    G722PLC_residu_flt(G722PLC_STATE_FLT * plc_state);


/* lower-band synthesis (main subroutine: G722PLC_syn) */
static void  G722PLC_syn_flt(G722PLC_STATE_FLT * plc_state, Float * syn, Short NumSamples);
static Float  G722PLC_ltp_pred_1s_flt(Float* exc, Short t0, Short *jitter);
static void    G722PLC_ltp_syn_flt(G722PLC_STATE_FLT* plc_state, Float* cur_exc, Float* cur_syn, Short n, Short *jitter);
static void    G722PLC_syn_filt_flt(Short m, Float* a, Float* x, Float* y, Short n);
static void  G722PLC_attenuate_flt(G722PLC_STATE_FLT * plc_state, Float * cur_sig, Float * tabout, Short NumSamples, 
                               Short * ind, Float * weight);
static void  G722PLC_attenuate_lin_flt(G722PLC_STATE_FLT * plc_state, Float fact, Float * cur_sig, Float * tabout, Short NumSamples, 
                                   Short * ind, Float * weight);
static void    G722PLC_calc_weight_flt(Short *ind_weight, Float fact1, Float fact2p, Float fact3p, Float * weight);
static void    G722PLC_update_mem_exc_flt(G722PLC_STATE_FLT * plc_state, Float * cur_sig, Short NumSamples);


/* higher-band synthesis */
static Float* G722PLC_syn_hb_flt(G722PLC_STATE_FLT * plc_state);

static void G722PLC_qmf_updstat_flt ARGS((Short *outcode, g722_state *decoder,
                                     Float *lb_signal, Float *hb_signal, void *plc_state));


/*================should be moved to lpctool.c===========================*/

/*----------------------------------------------------------*
* Function Lag_window_flt()                                    *
*                                                          *
* r[i] *= lag_wind[i]                                      *
*                                                          *
*    r[i] and lag_wind[i] are in special double precision. *
*    See "oper_32b.c" for the format                       *
*                                                          *
*----------------------------------------------------------*/

void Lag_window_flt(
                Float * R,
                const Float * W,
                int ord
                )
{
  int  i;

  for (i = 1; i <= ord; i++)
  {
    R[i] *= W[i - 1];
  }
  return;
}

void Levinson_flt(
              Float R[],     /* (i)     : R[M+1] Vector of autocorrelations  */
              Float rc[],      /* (o)   : rc[M]   Reflection coefficients.         */
              Short *stable,  /* (o)    : Stability flag                           */
              Short ord,       /* (i)   : LPC order                                */
              Float * a        /* (o)   : LPC coefficients                         */
              )
{
  Float  err, s, at ;                     /* temporary variable */
  int   i, j, l;

  *stable = 0; 

  /* K = A[1] = -R[1] / R[0] */
  rc[0] = (-R[1]) / R[0];
  a[0] = 1;
  a[1] = rc[0];
  err = R[0] + R[1] * rc[0];
  
  /*-------------------------------------- */
  /* ITERATIONS  I=2 to lpc_order          */
  /*-------------------------------------- */
  for (i = 2; i <= ord; i++) {
	  s = 0;
	  for (j = 0; j < i; j++) {
		  s += R[i - j] * a[j];
	  }
	  rc[i - 1] = -s/err;
	  /* Test for unstable filter. If unstable keep old A(z) */
	  if(fabs(rc[i-1])> 0.99) {
		  *stable = 1; 
		  return;
	  }

	  for (j = 1; j <= (i / 2); j++) {
		  l = i - j;
		  at = a[j] + rc[i - 1] * a[l];
		  a[l] += rc[i - 1] * a[j];
		  a[j] = at;
	  }
	  a[i] = rc[i - 1];
	  err += rc[i - 1] * s;
	  if (err <=  0) {
		  err = 0.001f;
	  }
  }
  return;
}

/*================should be moved to lpctool.c END===========================*/

void set_att_flt(G722PLC_STATE_FLT * plc_state, Short inc_att_v, Float fact1_v, Float fact2p_v, Float fact3p_v)
{
  plc_state->s_inc_att = inc_att_v;  
  plc_state->f_fact1 = fact1_v; 
  plc_state->f_fact2p = fact2p_v; 
  plc_state->f_fact3p = fact3p_v; 
}


/***********************************
* definition of main PLC routines *
***********************************/

/*----------------------------------------------------------------------
* G722PLC_init(l_frame)
* allocate memory and return PLC state variables
*
* l_frame (i) : frame length @ 8kHz
*---------------------------------------------------------------------- */
void * G722PLC_init_flt(void)
{
  G722PLC_STATE_FLT * plc_state;

  /* allocate memory for PLC plc_state */
  plc_state = (G722PLC_STATE_FLT *)malloc(sizeof(G722PLC_STATE_FLT));
  if(plc_state == NULL)
  {
    exit(-1);
  }

  /* LPC, pitch, signal classification parameters */
  plc_state->f_a = (Float *)calloc(ORD_LPC + 1, sizeof(Float));
  plc_state->f_mem_syn = (Float *)calloc(ORD_LPC, sizeof(Float));

  zeroFloat(ORD_LPC, plc_state->f_mem_syn);
  zeroFloat(ORD_LPCP1, plc_state->f_a);
  plc_state->s_clas = G722PLC_WEAKLY_VOICED;   

  /* signal buffers */
  plc_state->f_mem_speech = (Float *)calloc(MEMSPEECH_LEN, sizeof(Float));
  plc_state->f_mem_speech_hb = (Float *)calloc(LEN_HB_MEM, sizeof(Float)); /*MAXPIT is needed, for complexity reason; LEN_HB_MEM: framelength 20ms*/
  plc_state->f_mem_exc = (Float *)calloc(MAXPIT2P1, sizeof(Float));

  zeroFloat(MEMSPEECH_LEN, plc_state->f_mem_speech);
  zeroFloat(LEN_HB_MEM, plc_state->f_mem_speech_hb);
  zeroFloat(MAXPIT2P1, plc_state->f_mem_exc);

  /* cross-fading */
  plc_state->s_count_crossfade = CROSSFADELEN; 

  /* higher-band hig-pass filtering */
  /* adaptive muting */
  plc_state->f_weight_lb = 1;  
  plc_state->f_weight_hb = 1;  
  plc_state->s_inc_att = 1;  
  plc_state->f_fact1 = F_FACT1_V;  
  plc_state->f_fact2p = F_FACT2P_V;  
  plc_state->f_fact3p = F_FACT3P_V;  

  plc_state->f_mem_hpf_in = 0;
  plc_state->f_mem_hpf_out = 0;
  zeroFloat(CROSSFADELEN, plc_state->f_crossfade_buf);
  plc_state->s_count_att = 0; 
  plc_state->s_count_att_hb = 0; 
  plc_state->s_t0 = 0; 
  plc_state->s_t0p2 = 0; 
  plc_state->s_prev_bfi = 0; 

  return((void *)plc_state);
}



/*----------------------------------------------------------------------
* G722PLC_conceal_flt(plc_state, xl, xh, outcode, decoder)
* extrapolation of missing frame
*
* plc_state (i/o) : state variables of PLC
* xl  (o) : decoded lower-band
* xh  (o) : decoder higher-band
* outcode (o) : decoded synthesis
* decoder (i/o) : g722 states (QMF, ADPCM)
*---------------------------------------------------------------------- */
void G722PLC_conceal_flt(void * state, Short* outcode, g722_state *decoder)
{
  G722PLC_STATE_FLT * plc_state = (G722PLC_STATE_FLT *) state;
  int i;
  Float * xl, * xh, Temp;

  /***********************
  * reset counter *
  ***********************/

  plc_state->s_count_crossfade = 0;  /* reset counter for cross-fading */
  
  /***********************
  * generate lower band *
  ***********************/

  /* check if first missing frame (i.e. if previous frame received)
  first missing frame -> analyze past buffer + PLC 
  otherwise -> PLC
  */
  xl = &plc_state->f_mem_speech[257]; /*257 : MEMSPEECH_LEN, L_FRAME_NB*/
  if(plc_state->s_prev_bfi == 0)
  {
    plc_state->s_count_att = 0;   /* reset counter for attenuation in lower band */
    plc_state->s_count_att_hb = 0;  /* reset counter for attenuation in higher band */
    plc_state->f_weight_lb = 1;  
    plc_state->f_weight_hb = 1; 

    /**********************************
    * analyze buffer of past samples *
    * - LPC analysis
    * - pitch estimation
    * - signal classification
    **********************************/

    G722PLC_ana_flt(plc_state, decoder);

    /******************************
    * synthesize missing samples *
    ******************************/

    /* set increment for attenuation */
    if(plc_state->s_clas == G722PLC_VUV_TRANSITION)
    {
      /* attenuation in 30 ms */
      set_att_flt(plc_state, 2, F_FACT1_UV, F_FACT2P_UV, F_FACT3P_UV);
      Temp = F_FACT3_UV; 
    }
	else
    {
      set_att_flt(plc_state, 1, F_FACT1_V, F_FACT2P_V, F_FACT3P_V);
      Temp = F_FACT2_V; 
    }

    if(plc_state->s_clas == G722PLC_TRANSIENT)
    {
      /* attenuation in 10 ms */
      set_att_flt(plc_state, 6, F_FACT1_V_R, F_FACT2P_V_R, F_FACT3P_V_R);
      Temp = 0; 
    }

    /* synthesize lost frame, high band */
    xh = G722PLC_syn_hb_flt(plc_state);

    /*shift low band*/
    movF(257, &plc_state->f_mem_speech[L_FRAME_NB], plc_state->f_mem_speech); /*shift low band*/

    /* synthesize lost frame, low band directly to plc_state->mem_speech*/
    G722PLC_syn_flt(plc_state, xl, L_FRAME_NB);
    for(i = 1; i <= 8; i++)
    {
      plc_state->f_a[i] *= f_G722PLC_gamma_az[i];	
    }
    /* synthesize cross-fade buffer (part of future frame)*/
    G722PLC_syn_flt(plc_state, plc_state->f_crossfade_buf, CROSSFADELEN);

    /* attenuate outputs */
    G722PLC_attenuate_lin_flt(plc_state, plc_state->f_fact1, xl, xl, L_FRAME_NB, &plc_state->s_count_att, &plc_state->f_weight_lb);
    if(plc_state->s_clas == G722PLC_TRANSIENT)
    {
      plc_state->f_weight_lb = 0;
      
    }
    /*5 ms frame, xfadebuff in 2 parts*/
    G722PLC_attenuate_lin_flt(plc_state, plc_state->f_fact1, plc_state->f_crossfade_buf, plc_state->f_crossfade_buf, CROSSFADELEN/2, &plc_state->s_count_att, &plc_state->f_weight_lb);
    G722PLC_attenuate_lin_flt(plc_state, Temp, plc_state->f_crossfade_buf+L_FRAME_NB, plc_state->f_crossfade_buf+L_FRAME_NB, CROSSFADELEN/2, &plc_state->s_count_att, &plc_state->f_weight_lb);
    G722PLC_attenuate_lin_flt(plc_state, plc_state->f_fact1, xh, xh, L_FRAME_NB, &plc_state->s_count_att_hb, &plc_state->f_weight_hb);
  }
  else
  {
    movF(257, &plc_state->f_mem_speech[L_FRAME_NB], plc_state->f_mem_speech); /*shift*/
    /* copy samples from cross-fading buffer (already generated in previous bad frame decoding)  */

    movF(L_FRAME_NB, plc_state->f_crossfade_buf, xl);
    movF(L_FRAME_NB, &plc_state->f_crossfade_buf[L_FRAME_NB], plc_state->f_crossfade_buf); /*shift*/

    /* synthesize 2nd part of cross-fade buffer (part of future frame) and attenuate output */
    G722PLC_syn_flt(plc_state, plc_state->f_crossfade_buf+L_FRAME_NB, L_FRAME_NB);
    G722PLC_attenuate_flt(plc_state, plc_state->f_crossfade_buf+L_FRAME_NB, plc_state->f_crossfade_buf+L_FRAME_NB, L_FRAME_NB, &plc_state->s_count_att, &plc_state->f_weight_lb);
    xh = G722PLC_syn_hb_flt(plc_state);
    G722PLC_attenuate_flt(plc_state, xh, xh, L_FRAME_NB, &plc_state->s_count_att_hb, &plc_state->f_weight_hb);
  }

  /*****************************************
  * QMF synthesis filter and plc_state update *
  *****************************************/

  G722PLC_qmf_updstat_flt(outcode, decoder, xl, xh, plc_state);

  return;
}


/*----------------------------------------------------------------------
* G722PLC_clear(plc_state)
* free memory and clear PLC plc_state variables
*
* plc_state (i) : PLC state variables
*---------------------------------------------------------------------- */
void G722PLC_clear_flt(void * state)
{
  G722PLC_STATE_FLT * plc_state = (G722PLC_STATE_FLT *) state;

  free(plc_state->f_mem_speech);
  free(plc_state->f_mem_speech_hb);
  free(plc_state->f_mem_exc);
  free(plc_state->f_a);
  free(plc_state->f_mem_syn);
  free(plc_state);

}




/*********************************
* definition of PLC subroutines *
*********************************/

/*----------------------------------------------------------------------
* G722PLC_hp_flt(x1, y1_lo, y2_hi, signal)
*  high-pass filter
*
* x1          (i/o) : filter memory
* y1_hi,y1_lo (i/o) : filter memory
* signal     (i)   : input sample
*----------------------------------------------------------------------*/

Float G722PLC_hp_flt(Float *x1, Float* y1, Float signal, 
                  const Float *b_hp, const Float *a_hp)
{
  Float    ACC0;

  /*  y[i] =      x[i]   -         x[i-1]    */
  /*                     + 123/128*y[i-1]    */
  ACC0 = signal * b_hp[0] + *x1 * b_hp[1] + *y1 * a_hp[1];
  *x1 = signal;
  *y1 = ACC0;

  return(ACC0);
}



/*----------------------------------------------------------------------
* G722PLC_syn_hb_flt(plc_state, xh, n)
* reconstruct higher-band by pitch prediction
*
* plc_state (i/o) : plc_state variables of PLC
*---------------------------------------------------------------------- */

static Float* G722PLC_syn_hb_flt(G722PLC_STATE_FLT* plc_state)
{
  Float *ptr;
  Float *ptr2;
  Short loc_t0;
  Short   tmp;

  /* save pitch delay */
  loc_t0 = plc_state->s_t0;
  
  /* if signal is not voiced, cut harmonic structure by forcing a 10 ms pitch */
  if(plc_state->s_clas != G722PLC_VOICED) /*constant G722PLC_VOICED = 0*/
  {
    loc_t0 = 80;
  }

  if(plc_state->s_clas == G722PLC_UNVOICED)/*G722PLC_UNVOICED*/
  {
    Float mean;
    Short tmp1, i;

    mean = 0;
    
    tmp1 = LEN_HB_MEM - 80; /* tmp1 = start index of last 10 ms, last periode is smoothed */
    for(i = 0; i < 80; i++)
    {
      mean += (Float)fabs(plc_state->f_mem_speech_hb[tmp1 + i]);
    }
    mean /= 32;  /*80/32 = 2.5 mean amplitude*/

    tmp1 = LEN_HB_MEM - loc_t0; /* tmp1 = start index of last periode that is smoothed */
    for(i = 0; i < loc_t0; i++)
    {
      if(fabs(plc_state->f_mem_speech_hb[tmp1 + i]) > mean)
      {
        plc_state->f_mem_speech_hb[tmp1 + i] /= 4; 
      }
    }
  }

  /* reconstruct higher band signal by pitch prediction */
  tmp = L_FRAME_NB - loc_t0;
  ptr = plc_state->f_mem_speech_hb + LEN_HB_MEM - loc_t0; /*beginning of copy zone*/
  ptr2 = plc_state->f_mem_speech_hb + LEN_HB_MEM_MLF; /*beginning of last frame in mem_speech_hb*/
  if(tmp <= 0) /* l_frame <= t0*/
  {
    /* temporary save of new frame in plc_state->mem_speech[0 ...L_FRAME_NB-1] of low_band!! that will be shifted after*/
    movF(L_FRAME_NB, ptr, plc_state->f_mem_speech);
    movF(LEN_HB_MEM_MLF, &plc_state->f_mem_speech_hb[L_FRAME_NB], plc_state->f_mem_speech_hb); /*shift 1 frame*/

    movF(L_FRAME_NB, plc_state->f_mem_speech, ptr2);
  }
  else /*t0 < L_FRAME_NB*/
  {
    movF(LEN_HB_MEM_MLF, &plc_state->f_mem_speech_hb[L_FRAME_NB], plc_state->f_mem_speech_hb); /*shift memory*/

    movF(loc_t0, ptr, ptr2); /*copy last period*/
    movF(tmp, ptr2, &ptr2[loc_t0]); /*repeate last period*/
  }
  return(ptr2);
}



/*-------------------------------------------------------------------------*
* G722PLC_attenuate_flt(state, in, out, n, count, weight)
* linear muting with adaptive slope
*
* state (i/o) : PLC state variables
* in    (i)   : input signal
* out (o)   : output signal = attenuated input signal
* n   (i)   : number of samples
* count (i/o) : counter
* weight (i/o): muting factor
*--------------------------------------------------------------------------*/
static void G722PLC_attenuate_flt(G722PLC_STATE_FLT* plc_state, Float* in, Float* out, Short n, Short *count, Float * weight)
{
  Short i;

  for (i = 0; i < n; i++)
  {
    /* calculate attenuation factor and multiply */
    G722PLC_calc_weight_flt(count, plc_state->f_fact1, plc_state->f_fact2p, plc_state->f_fact3p, weight);
    out[i] = *weight * in[i];
    
    *count += plc_state->s_inc_att;
  }
  return;
}


/*-------------------------------------------------------------------------*
* G722PLC_attenuate_lin_flt(plc_state, fact, in, out, n, count, weight)
* linear muting with fixed slope
*
* plc_state (i/o) : PLC state variables
* fact  (i/o) : muting parameter
* in    (i)   : input signal
* out (o)   : output signal = attenuated input signal
* n   (i)   : number of samples
* count (i/o) : counter
* weight (i/o): muting factor
*--------------------------------------------------------------------------*/

static void G722PLC_attenuate_lin_flt(G722PLC_STATE_FLT* plc_state, Float fact, Float* in, Float* out, Short n, Short *count, Float * weight)
{
  Short i;

  for (i = 0; i < n; i++) /*adaptation 5ms*/
  {
    /* calculate attenuation factor and multiply */
    *weight = *weight - fact;
    out[i] = *weight * in[i];
  }
  *count += plc_state->s_inc_att * n; /*adaptation 5ms*/

  return;
}



/*-------------------------------------------------------------------------*
* G722PLC_calc_weight_flt(ind_weight, Ind1, Ind12, Ind123, Rlim1, Rlim2, fact1, fact2, fact3,
*                     tab_len, WeightEnerSynthMem)
* calculate attenuation factor
*--------------------------------------------------------------------------*/

static void G722PLC_calc_weight_flt(Short *ind_weight, Float fact1, Float fact2p, Float fact3p, Float * weight)
{

  *weight -= fact1;
  if (*ind_weight >= END_1ST_PART)
  {
    *weight = *weight - fact2p;
  }
  if (*ind_weight >= END_2ND_PART)
  {
    *weight = *weight - fact3p;
  }
  if (*ind_weight >= END_3RD_PART)
  {
    *weight = 0;   
  }
  if(*weight <= 0)
  {
    *ind_weight = END_3RD_PART;   
  }
  return;
}



/*-------------------------------------------------------------------------*
* Function G722PLC_update_mem_exc_flt                                          *
* Update of plc_state->mem_exc and shifts the memory                           *
* if plc_state->t0 > L_FRAME_NB                                            *
*--------------------------------------------------------------------------*/
static void G722PLC_update_mem_exc_flt(G722PLC_STATE_FLT * plc_state, Float * exc, Short n)
{
  Float *ptr;
  Short temp;

  /* shift ResMem, if t0 > l_frame */
  temp = plc_state->s_t0p2 - n;

  ptr = plc_state->f_mem_exc + MAXPIT2P1 - plc_state->s_t0p2;
  if (temp > 0)
  {
    movF(temp, &ptr[n], ptr);
    movF(n, exc, &ptr[temp]);
  }
  else
  {
    /* copy last "pitch cycle" of residual */
    movF(plc_state->s_t0p2, &exc[n - plc_state->s_t0p2], ptr);
  }
  return;
}



/*-------------------------------------------------------------------------*
* Function G722PLC_ana_flt(plc_state, decoder)                   *
* Main analysis routine
*
* plc_state   (i/o) : PLC state variables
* decoder (i)   : G.722 decoder state variables
*-------------------------------------------------------------------------*/

static void G722PLC_ana_flt(G722PLC_STATE_FLT * plc_state, g722_state *decoder)
{  
  Float maxco;
  Float nooffsig[MEMSPEECH_LEN];
  int i;

  Float x1, y1;

  /* DC-remove filter */
  x1 = y1 = 0;
   
  /* DC-remove filter */
  for(i = 0; i < MEMSPEECH_LEN; i++)
  {
    nooffsig[i] = G722PLC_hp_flt(&x1, &y1, plc_state->f_mem_speech[i],
      f_G722PLC_b_hp, f_G722PLC_a_hp);
  }

  /* perform LPC analysis and compute residual signal */
  G722PLC_lpc_flt(plc_state, nooffsig);
  G722PLC_residu_flt(plc_state);
  /* estimate (open-loop) pitch */
  /* attention, may shift noofsig, but only used after for zero crossing rate not influenced by this shift (except for very small values)*/
  plc_state->s_t0 = G722PLC_pitch_ol_flt(nooffsig + MEMSPEECH_LEN - MAXPIT2, &maxco);

  /* update memory for LPC
  during ereased period the plc_state->mem_syn contains the non weighted
  synthetised speech memory. For thefirst erased frame, it
  should contain the output speech.
  Saves the last ORD_LPC samples of the output signal in
  plc_state->mem_syn    */
  movF(ORD_LPC, &plc_state->f_mem_speech[MEMSPEECH_LEN - ORD_LPC],
    plc_state->f_mem_syn);

  /* determine signal classification and modify residual in case of transient */
  plc_state->s_clas = G722PLC_classif_modif_flt(maxco, decoder->nbl, decoder->nbh, nooffsig, MEMSPEECH_LEN,
    plc_state->f_mem_exc, &plc_state->s_t0);

  plc_state->s_t0p2 = plc_state->s_t0 + 2; 

  return;
}



/*-------------------------------------------------------------------------*
* Function G722PLC_autocorr_flt*
*--------------------------------------------------------------------------*/

void G722PLC_autocorr_flt(Float x[],  /* (i)    : Input signal                      */
                      Float r[],/* (o)    : Autocorrelations  (msb)           */
                      Short ord,    /* (i)    : LPC order                         */
                      Short len /* (i)    : length of analysis                   */
                      )
{
  Float sum;
  Float *y = (Float *)calloc(len, sizeof(Float));
  int i, j;

  /* Windowing of signal */
  for(i = 0; i < len; i++)
  {
    y[i] = x[i] * f_G722PLC_lpc_win_80[HAMWINDLEN-len+i]; /* for length < 80, uses the end of the window */
  }
  /* Compute r[0] and test for overflow */

  for (i = 0; i <= ord; i++) {
    sum = 0;
    for (j = 0; j < len - i; j++) {
      sum += y[j] * y[j + i];
    }
    r[i] = sum;
  }
  free(y);

  return;
}



static void G722PLC_pitch_ol_refine_flt(Float * nooffsigptr, int il, Float ener1_f,  
                                    int beg_last_per, int end_last_per,
                                    int *ind, Float *maxco)
{
  int i, j; 
  Float corx_f, ener2_f;
  int start_ind, end_ind;
  Float em;

  start_ind = il - 2;
  if (start_ind < MINPIT) {
    start_ind = MINPIT;
  }
  end_ind = il + 2;
  j = end_last_per - start_ind;
  ener2_f = 1 + nooffsigptr[j] * nooffsigptr[j]; /*to avoid division by 0*/
  for(j = end_last_per - 1; j > beg_last_per; j--)
  {
    ener2_f += nooffsigptr[j-start_ind] * nooffsigptr[j-start_ind];
  }
  for(i = start_ind; i <= end_ind; i++)
  {
    corx_f = 0;   

    ener2_f += nooffsigptr[beg_last_per-i] * nooffsigptr[beg_last_per-i]; /*update, part 2*/
    for(j = end_last_per; j >= beg_last_per; j--)
    {
      corx_f += nooffsigptr[j] * nooffsigptr[j-i];
    }
    if (ener1_f > ener2_f) {
      em = ener1_f;
    }
    else {
      em = ener2_f;
    }
    if (corx_f > 0) {
      corx_f /= em;
    }
    if (corx_f > *maxco) {
      *ind = i;
      *maxco = corx_f;
    }
    ener2_f -= nooffsigptr[end_last_per-i] - nooffsigptr[end_last_per-i]; /*update, part 1*/
  }
  return;
}



/*----------------------------------------------------------------------
* G722PLC_pitch_ol_flt(signal, length, maxco)
* open-loop pitch estimation
*
* signal      (i) : pointer to signal buffer (including signal memory)
* length      (i) : length of signal memory
* maxco       (o) : maximal correlation
* overlf_shft (o) : number of shifts
*
*---------------------------------------------------------------------- */

static Short G722PLC_pitch_ol_flt(Float * signal, Float *maxco)
{  
  int i, j, il, k; 
  int ind, ind2;
  Short zcr, stable;
  int beg_last_per;
  int valid = 0; /*not valid for the first lobe */
  int previous_best;

  Float *w_ds_sig;
  Float corx_f, ener1_f, ener2_f;
  Float temp_f;
  Float em;
  Float *ptr1, *nooffsigptr;
  Float L_temp; 

  Float ds_sig[MAXPIT2_DS];
  Float ai[3], cor[3], rc[3];
  Float *pt1, *pt2;
  Float *w_ds_sig_alloc;

  nooffsigptr = signal; /* 296 - 8; rest MAXPIT2 */

  /* downsample (filter and decimate) signal */
  ptr1 = ds_sig;
  for(i = FACT_M1; i < MAXPIT2; i += FACT)
  {
    temp_f = nooffsigptr[i] * f_G722PLC_fir_lp[0];
    pt2 = nooffsigptr+i;
    for (k = 1; k < FEC_L_FIR_FILTER_LTP; k++)
    {
      pt2--;
      temp_f += *pt2 * f_G722PLC_fir_lp[k];
    }
    *ptr1++ = temp_f;
  }

  G722PLC_autocorr_flt(ds_sig, cor, 2, MAXPIT2_DS);
  Lag_window_flt(cor, f_G722PLC_lag, 2);/* Lag windowing*/
  Levinson_flt(cor, rc, &stable, 2, ai);
  ai[1] *= F_GAMMA;
  ai[2] *= F_GAMMA2;

  /* filter */
  w_ds_sig_alloc = (Float *)calloc(MAXPIT2_DS-2, sizeof(Float));
  w_ds_sig = w_ds_sig_alloc-2;

  for (i = 2; i < MAXPIT2_DS; i++)
  {
    L_temp = ai[1] * ds_sig[i - 1] + ai[2] * ds_sig[i - 2];	
    w_ds_sig[i] = ds_sig[i] + L_temp;   
  }

  ind = MAXPIT_S2_DS;  /*default value, 18*/
  previous_best = 0;   /*default value*/
  ind2 = 1;  

  /* compute energy of signal in range [len/fac-1,(len-MAX_PIT)/fac-1] */
  ener1_f = 1;
  for (j = MAXPIT2_DSM1; j >= MAXPIT_DSP1; j--) {
    ener1_f += w_ds_sig[j] * w_ds_sig[j];
  }

  /* compute maximal correlation (maxco) and pitch lag (ind) */
  *maxco = 0;  
  ener2_f = ener1_f - w_ds_sig[MAXPIT2_DSM1] * w_ds_sig[MAXPIT2_DSM1]; /*update, part 1*/
  ener2_f += w_ds_sig[MAXPIT_DSP1-1] * w_ds_sig[MAXPIT_DSP1-1]; /*update, part 2*/
  ener2_f -= w_ds_sig[MAXPIT2_DSM1-1] * w_ds_sig[MAXPIT2_DSM1-1]; /*update, part 1*/
  pt1 = &w_ds_sig[MAXPIT2_DSM1];
  pt2 = pt1-1;	
  zcr = 0;  
  for(i = 2; i < MAXPIT_DS; i++) /* < to avoid out of range later*/
  {
    ind2 += 1;
    corx_f = 0;    

    for(j = MAXPIT2_DSM1; j >= MAXPIT_DSP1; j--)
    {
      corx_f += w_ds_sig[j] * w_ds_sig[j-i];
    }
    ener2_f += w_ds_sig[MAXPIT_DSP1-i] * w_ds_sig[MAXPIT_DSP1-i]; /*update, part 2*/
    if (ener1_f > ener2_f) {
      em = ener1_f;
    }
    else {
      em = ener2_f;
    }
    ener2_f -= w_ds_sig[MAXPIT2_DSM1-i] * w_ds_sig[MAXPIT2_DSM1-i]; /*update, part 1*/
    if (corx_f > 0) {
      corx_f /= em;
    }

    if (corx_f < 0) {
      valid = 1;
      /* maximum correlation is only searched after the first positive lobe of autocorrelation function */
    }
    /* compute (update)zero-crossing  in last examined period */
    if (((int)*pt1 ^ (int)*pt2) < 0) {
      zcr++;
    }
    pt1--;
    pt2--;
    if(zcr == 0) /*no zero crossing*/
    {
      valid = 0;     
    }

    if(valid > 0)
    {
      if ((ind2 == ind) || (ind2 == 2 * ind)) { /* double or triple of actual pitch */
        if (*maxco > 0.85) {  /* 0.85 : high correlation, small chance that double pitch is OK */
          *maxco = 1;         /* the already found pitch value is kept */
        }

        if(*maxco < 0.888855f)/*to avoid overflow*/
        {
          *maxco *= (Float) 1.125; 
        }
      }

      if ((corx_f > *maxco) && (i >= MINPIT_DS)) {
        *maxco = corx_f;
        if(i != (ind+1))
        {
          previous_best = ind; 
        }
        ind = i;                /*save the new candidate */
        ind2 = 1;               /* reset counter for multiple pitch */
      }
    }
  }
  free(w_ds_sig_alloc);

  /* convert pitch to non decimated domain */
  il = 4 * ind;
  ind = il;  

  /* refine pitch in non-decimated (8 kHz) domain by step of 1
  -> maximize correlation around estimated pitch lag (ind) */
  beg_last_per = MAXPIT2 - il;
  ener1_f = 1 + nooffsigptr[END_LAST_PER] * nooffsigptr[END_LAST_PER]; /*to avoid division by 0*/
  for(j = END_LAST_PER_1; j >= beg_last_per; j--)
  {
    ener1_f += nooffsigptr[j] * nooffsigptr[j];
  }
  /* compute maximal correlation (maxco) and pitch lag (ind) */
  *maxco = 0;  

  G722PLC_pitch_ol_refine_flt(nooffsigptr, il, ener1_f, beg_last_per, END_LAST_PER, &ind,  maxco);
  if(*maxco < 0.4) /*check 2nd candidate if maxco > 0.4*/
  {
    previous_best = 0;    
  }
  if(previous_best != 0) /*check second candidate*/
  {
    il = 4 * previous_best;
    G722PLC_pitch_ol_refine_flt(nooffsigptr, il, ener1_f, beg_last_per, END_LAST_PER, &ind,  maxco);
  }

  if ((*maxco < 0.25) && (ind < 32))
  {
	  ind *= 2; /*2 times pitch for very small pitch, at least 2 times MINPIT */
  }

  if (*maxco > 1) {
    *maxco = 1;
  }

  return ind;
}



/*----------------------------------------------------------------------
* G722PLC_classif_modif_flt(maxco, decoder)
* signal classification and conditional residual modification
*
* maxco       (i) : maximal correlation
* nbl         (i) : lower-band G722 scale factor
* nbh         (i) : higher-band G722 scale factor
* mem_speech  (i) : pointer to speech buffer
* l_mem_speech(i) : length of speech buffer
* mem_exc     (i) : pointer to excitation buffer
* t0          (i) : open-loop pitch
*---------------------------------------------------------------------- */

static Short G722PLC_classif_modif_flt(Float maxco, Short nbl, Short nbh, Float* mem_speech, int l_mem_speech,
                                    Float* mem_exc, Short* t0
                                    )
{
  Short clas, zcr;
  int Temp, tmp1, tmp2, tmp3, i, j;
  Float  maxres, absres;
  Float *pt1, *pt2, ftmp;

  /************************************************************************
  * select preliminary class => clas = UNVOICED, WEAKLY_VOICED or VOICED *
  * by default clas=WEAKLY_VOICED                         *
  * classification criterio:                                             *
  * -normalized correlation from open-loop pitch                         *
  * -ratio of lower/higher band energy (G722 scale factors)              *
  * -zero crossing rate                                                  *
  ************************************************************************/

  /* compute zero-crossing rate in last 10 ms */
  pt1 = &mem_speech[l_mem_speech - 80];
  pt2 = pt1-1;	
  zcr = 0;
  
  for(i = 0; i< 80; i++)
  {
    if((*pt1 <= 0) && (*pt2 > 0))
    {
      zcr++;
    }
    pt1++;
    pt2++;
  }

  /* set default clas */
  clas = G722PLC_WEAKLY_VOICED;
  
  /* detect voiced clas (corr > 3/4 ener1 && corr > 3/4 ener2) */
  if(maxco > 0.7) 
  {
    clas = G722PLC_VOICED;
  }

  /* change class to unvoiced if higher band has lots of energy
  (risk of "dzing" if clas is "very voiced") */
  if(nbh > nbl)
  {
    if(clas == G722PLC_VOICED)  /*if the class is VOICED (constant G722PLC_VOICED = 0)*/
    {
		clas = G722PLC_WEAKLY_VOICED;
    }
	else
	{
		clas = G722PLC_VUV_TRANSITION;
	}
  }

  /* change class to unvoiced if zcr is high */
  if (zcr >= 20)
  {
    clas = G722PLC_UNVOICED;
    
    /* change pitch if unvoiced class (to avoid short pitch lags) */
    if(*t0 < 32)
    {
      *t0 *= 2; /*2 times pitch for very small pitch, at least 2 times MINPIT */
    }
  }


  /**************************************************************************
  * detect transient => clas = TRANSIENT                                  *
  * + modify residual to limit amplitude for LTP                           *
  * (this is performed only if current class is not VOICED to avoid        *
  *  perturbation of the residual for LTP)                                 *
  **************************************************************************/

  /* detect transient and limit amplitude of residual */
  Temp = 0;
  if (clas > 4)/*G722PLC_WEAKLY_VOICED(5) or G722PLC_VUV_TRANSITION(7)*/
  {
    tmp1 = MAXPIT2P1 - *t0; /* tmp1 = start index of last "pitch cycle" */
    tmp2 = tmp1 - *t0;  /* tmp2 = start index of last but one "pitch cycle" */
    for(i = 0; i < *t0; i++)
    {
      tmp3 = tmp2 + i;
	  maxres = (Float)fabs(mem_exc[tmp3-2]);
	  for(j = -1; j <=2; j++)
	  {
		  ftmp = (Float)fabs(mem_exc[tmp3+j]);
		  if(ftmp > maxres)
		  {
			  maxres = ftmp;
		  }
	  }
      absres = (Float)fabs(mem_exc[tmp1 + i]);	

      /* if magnitude in last cycle > magnitude in last but one cycle */
      if(absres > maxres)
      {
        /* detect transient (ratio = 1/8) */
        if(absres >= 8*maxres) 
        {
          Temp++;
        }

        /* limit value (even if a transient is not detected...) */
        if(mem_exc[tmp1 + i] < 0)	
        {
          mem_exc[tmp1 + i] = -maxres;	
        }
		else
        {
          mem_exc[tmp1 + i] = maxres;	
        }
      }
    }
  }
  if(clas == G722PLC_UNVOICED)/*G722PLC_UNVOICED*/
  {
    Float mean;

    mean = 0;
    
    /* 209 = MAXPIT2P1 - 80, start index of last 10 ms, last period is smoothed */
    for(i = 0; i < 80; i++)
    {
      mean += (Float)fabs(mem_exc[209 + i]);
    }
    mean /= 32;  /*80/32 = 2.5 mean amplitude*/

    tmp1 = MAXPIT2P1 - *t0; /* tmp1 = start index of last "pitch cycle" */

    for(i = 0; i < *t0; i++)
    {
      if(fabs(mem_exc[tmp1 + i]) > mean)
      {
        mem_exc[tmp1 + i] /= 4;	
      }
    }
  }
  if (Temp>0)
  {
    clas = G722PLC_TRANSIENT;
	if(*t0 > 40)
	{
      *t0 = 40; /*max 5 ms pitch */
	}
  }

  /*******************************************************************************
  * pitch tuning by searching last glotal pulses                                *
  * checks if there is no 2 pulses in the last periode due to decreasing pitch  *
  *******************************************************************************/

  if(clas == 0)  /*if the class is VOICED (constant G722PLC_VOICED = 0)*/
  {
    Float maxpulse; 
    int mincheck, pulseind=0;
    int end2nd;
    Float maxpulse2nd; 
    Short pulseind2nd=0; 
    Float absval;
    Float cumul, pulsecumul;
    Short signbit, signbit2nd;

    mincheck = *t0 - 5;
    maxpulse = -1;
    maxpulse2nd = -1;
    cumul = 0;
    pulsecumul = 0;

    pt1 = &mem_exc[MAXPIT2P1 - 1]; /*check the last period*/

    for(i = 0; i < *t0; i++) /*max pitch variation searched is +-5 */
    {
      absval = (Float)fabs(*pt1);
      if(absval > maxpulse) 
      {
		maxpulse = absval;
        pulseind = i;
      }
      cumul += absval;
      pt1--;
    }
    pulsecumul = maxpulse * *t0;
    if(mem_exc[MAXPIT2P1 - pulseind - 1] < 0) /*check the sign*/
	{
		signbit = 1;
	}
	else
	{
		signbit = 0;
	}

    if(cumul < pulsecumul/4) /* if mean amplitude < max amplitude/4 --> real pulse*/
    {
      end2nd = pulseind - mincheck;
      pt1 = &mem_exc[MAXPIT2P1 - 1]; /*end of excitation*/

      for(i = 0; i < end2nd; i++) /*search 2nd pulse at the end of the periode*/
      {
        absval = (Float)fabs(*pt1);  /*abs_s added on 25/07/2007*/
        if(absval > maxpulse2nd)
        {
	      maxpulse2nd = absval;
          pulseind2nd = i;
        }
        pt1--;
      }
      end2nd = mincheck + pulseind;
      pt1 = &mem_exc[MAXPIT2P1 - 1 - end2nd]; /*end of excitation*/

      for(i = end2nd; i < *t0; i++) /*search 2nd pulse at the beggining of the periode*/
      {
        absval = (Float)fabs(*pt1);
        if(absval > maxpulse2nd)
        {
	      maxpulse2nd = absval;
          pulseind2nd = i;
        }
        pt1--;
      }
      if(maxpulse2nd > maxpulse/2)
      {
		if(mem_exc[MAXPIT2P1 - pulseind2nd - 1] < 0) /*check the sign*/
		{
			signbit2nd = 1;
		}
		else
		{
			signbit2nd = 0;
		}

        if(signbit == signbit2nd)
        {
          *t0 = (Short)fabs(pulseind - pulseind2nd);
        }
      }
    }
  }

  return clas;
}



/*----------------------------------------------------------------------
* G722PLC_syn_filt_flt(m, a, x, y, n n)
* LPC synthesis filter
*
* m (i) : LPC order
* a (i) : LPC coefficients
* x (i) : input buffer
* y (o) : output buffer
* n (i) : number of samples
*---------------------------------------------------------------------- */

static void G722PLC_syn_filt_flt(Short m, Float* a, Float* x, Float* y, Short n)
{
  Short j;

  *y = a[0] * *x;
  for (j = 1; j <= m; j++)
  {
    *y -= a[j] * y[-j];
  }
  return;
}



/*----------------------------------------------------------------------
* G722PLC_ltp_pred_1s_flt(cur_exc, t0, jitter)
* one-step LTP prediction and jitter update
*
* exc     (i)   : excitation buffer (exc[...-1] correspond to past)
* t0      (i)   : pitch lag
* jitter  (i/o) : pitch lag jitter
*---------------------------------------------------------------------- */

static Float G722PLC_ltp_pred_1s_flt(Float* exc, Short t0, Short *jitter)
{
  Short i;

  i = *jitter - t0;

  /* update jitter for next sample */
  *jitter = -*jitter;

  /* prediction =  exc[-t0+jitter] */
  return exc[i];
}



/*----------------------------------------------------------------------
* G722PLC_ltp_syn_flt(plc_state, cur_exc, cur_syn, n, jitter)
* LTP prediction followed by LPC synthesis filter
*
* plc_state    (i/o) : PLC state variables
* cur_exc  (i)   : pointer to current excitation sample (cur_exc[...-1] correspond to past)
* cur_syn  (i/o) : pointer to current synthesis sample
* n     (i)      : number of samples
* jitter  (i/o)  : pitch lag jitter
*---------------------------------------------------------------------- */

static void G722PLC_ltp_syn_flt(G722PLC_STATE_FLT* plc_state, Float* cur_exc, Float* cur_syn, Short n, Short *jitter)
{
  Short i;

  for (i = 0; i < n; i++)
  {
    /* LTP prediction using exc[...-1] */
    *cur_exc = G722PLC_ltp_pred_1s_flt(cur_exc, plc_state->s_t0, jitter);

    /* LPC synthesis filter (generate one sample) */
    G722PLC_syn_filt_flt(ORD_LPC, plc_state->f_a, cur_exc, cur_syn, 1);

    cur_exc++;
    cur_syn++;
  } 

  return;
}



/*----------------------------------------------------------------------
* G722PLC_syn_flt(plc_state, syn, n)
* extrapolate missing lower-band signal (PLC)
*
* plc_state (i/o) : PLC state variables
* syn   (o)   : synthesis
* n     (i)   : number of samples
*---------------------------------------------------------------------- */

static void G722PLC_syn_flt(G722PLC_STATE_FLT * plc_state, Float * syn, Short n)
{
  Float *buffer_syn; /* synthesis buffer */
  Float *buffer_exc; /* excitation buffer */
  Float *cur_syn;    /* pointer to current sample of synthesis */
  Float *cur_exc;    /* pointer to current sample of excition */
  Float *exc;        /* pointer to beginning of excitation in current frame */
  Short temp;
  Short jitter, dim;

  dim = n + plc_state->s_t0p2;
  /* allocate temporary buffers and set pointers */
  buffer_exc = (Float *)calloc(dim, sizeof(Float));
  buffer_syn = (Float *)calloc(2*ORD_LPC, sizeof(Float)); /* minimal allocations of scratch RAM */

  cur_exc = &buffer_exc[plc_state->s_t0p2];	/* pointer ! */
  cur_syn = &buffer_syn[ORD_LPC]; /* pointer */

  exc = cur_exc;	/* pointer */

  /* copy memory
  - past samples of synthesis (LPC order)            -> buffer_syn[0]
  - last "pitch cycle" of excitation (t0+2) -> buffer_exc[0]
  */
  movF(ORD_LPC, plc_state->f_mem_syn, buffer_syn); /*  */

  movF(plc_state->s_t0p2, plc_state->f_mem_exc + MAXPIT2P1 - plc_state->s_t0p2, buffer_exc);	

  /***************************************************
  * set pitch jitter according to clas information *
  ***************************************************/
  jitter = plc_state->s_clas & 1;
  plc_state->s_t0 = plc_state->s_t0 | jitter;    /* change even delay as jitter is more efficient for odd delays */

  /*****************************************************
  * generate signal by LTP prediction + LPC synthesis *
  *****************************************************/

  temp = n - ORD_LPC;
  /* first samples [0...ord-1] */
  G722PLC_ltp_syn_flt(plc_state, cur_exc, cur_syn, ORD_LPC, &jitter);
  movF(ORD_LPC, cur_syn, syn);

  /* remaining samples [ord...n-1] */
  G722PLC_ltp_syn_flt(plc_state, &cur_exc[ORD_LPC], &syn[ORD_LPC], temp, &jitter);

  /* update memory:
  - synthesis for next frame (last LPC-order samples)
  - excitation */
  movF(ORD_LPC, &syn[temp], plc_state->f_mem_syn);
  G722PLC_update_mem_exc_flt(plc_state, exc, n);

  /* free allocated memory */
  free(buffer_syn);
  free(buffer_exc);

  return;
}



/*-------------------------------------------------------------------------*
* Function G722PLC_lpc_flt *
*--------------------------------------------------------------------------*/
static void G722PLC_lpc_flt(G722PLC_STATE_FLT * plc_state, Float * mem_speech)
{
  Short tmp;

  Float cor[ORD_LPC + 1];	
  Float rc[ORD_LPC + 1];	

  G722PLC_autocorr_flt(&mem_speech[MEMSPEECH_LEN - HAMWINDLEN], cor, ORD_LPC, HAMWINDLEN);	
  Lag_window_flt(cor, f_G722PLC_lag, ORD_LPC);/* Lag windowing*/
  Levinson_flt(cor, rc, &tmp, ORD_LPC, plc_state->f_a);

  return;
}



/*-------------------------------------------------------------------------*
* Function G722PLC_residu_flt *
*--------------------------------------------------------------------------*/
static void G722PLC_residu_flt(G722PLC_STATE_FLT * plc_state)
{
  Float L_temp;
  Float *ptr_sig, *ptr_res;
  int i, j;

  ptr_res = plc_state->f_mem_exc;
  ptr_sig = &plc_state->f_mem_speech[MEMSPEECH_LEN - MAXPIT2P1];	

  for (i = 0; i < MAXPIT2P1; i++)
  {
    L_temp = ptr_sig[i] * plc_state->f_a[0];
    for (j = 1; j <= ORD_LPC; j++)
    {
      L_temp += plc_state->f_a[j] * ptr_sig[i - j];	
    }
    ptr_res[i] = L_temp;
  }

  return;
}



#define DLT  decoder->dlt
#define PLT  decoder->plt
#define RLT  decoder->rlt
#define SL   decoder->sl
#define SZL  decoder->szl
#define DETL decoder->detl
#define NBL  decoder->nbl
#define DETH decoder->deth
#define NBH  decoder->nbh
#define AL   decoder->al
#define BL   decoder->bl



static void G722PLC_qmf_updstat_flt (outcode,decoder,lb_signal,hb_signal,state)
Short *outcode;
Float *lb_signal;
Float *hb_signal;
g722_state     *decoder;
void *state;

{
  Short  rh, rl;
  Short  i; 
  G722PLC_STATE_FLT * plc_state = (G722PLC_STATE_FLT *) state;
  Float *endLastOut; 
  Float *firstFuture;
  Short *filtmem, *filtptr;

  filtmem = (Short *)calloc(102, sizeof(Short));

  movSS(22, &decoder->qmf_rx_delayx[2], &filtmem[L_FRAME_WB]); /*load memory*/
  filtptr = &filtmem[L_FRAME_WB];
  for (i = 0; i < L_FRAME_NB; i++)
  {
    /* filter higher-band */
    rh = (Short)G722PLC_hp_flt(&plc_state->f_mem_hpf_in, &plc_state->f_mem_hpf_out, *hb_signal,
      f_G722PLC_b_hp156, f_G722PLC_a_hp156);
	rl = (Short)*lb_signal;
    /* calculate output samples of QMF filter */
    fl_qmf_rx_buf (rl, rh, &filtptr, &outcode);

    lb_signal++;
    hb_signal++;
  }

  movSS(22, filtmem, &decoder->qmf_rx_delayx[2]); /*save memory*/
  free(filtmem);

  /* reset G.722 decoder */ 
  endLastOut = &(plc_state->f_mem_speech[MEMSPEECH_LEN - 1]);	
  firstFuture = plc_state->f_crossfade_buf;

  zeroS(7, DLT);

  PLT[1] = (Short)(endLastOut[0]/2);
  PLT[2] = (Short)(endLastOut[-1]/2);

  RLT[1] = (Short)endLastOut[0];
  RLT[2] = (Short)endLastOut[-1];

  SL = (Short)firstFuture[0];
  SZL = (Short)(firstFuture[0]/2);

  /* change scale factors (to avoid overshoot) */
  NBH  = NBH >> 1;
  DETH = scaleh(NBH); 
  
  /* reset G.722 decoder after muting */
  if( plc_state->s_count_att_hb > 160 )
  {
    DETL = 32;
    NBL = 0;
    DETH = 8;
    NBH = 0;
  }
  AL[1] = (Short)(AL[1] * F_GAMMA_AL2);	 
  AL[2] = (Short)(AL[2] * F_GAMMA2_AL2);	 
  BL[1] = (Short)(BL[1] * F_GAMMA_AL2);	 
  BL[2] = (Short)(BL[2] * F_GAMMA2_AL2);	 
  BL[3] = (Short)(BL[3] * F_GAMMA3_AL2);	 
  BL[4] = (Short)(BL[4] * F_GAMMA4_AL2);	 
  BL[5] = (Short)(BL[5] * F_GAMMA5_AL2);	 
  BL[6] = (Short)(BL[6] * F_GAMMA6_AL2);	 

}


#undef DLT 
#undef PLT 
#undef RLT 
#undef SL  
#undef SZL 
#undef DETL
#undef NBL
#undef DETH
#undef DH 
#undef NBH
#undef AL
#undef BL

/* .................... end of G722PLC_qmf_updstat() .......................... */
