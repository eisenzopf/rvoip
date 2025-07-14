/* ITU G.722 3rd Edition (2012-09) */

/*--------------------------------------------------------------------------
 ITU-T Annex B (ex G.722-SWB) Source Code
 Software Release 1.00 (2010-09)
 (C) 2010 France Telecom, Huawei Technologies, NTT, VoiceAge Corp.
--------------------------------------------------------------------------*/

#include "pcmswb_common.h"
#include "oper_32b.h" 
#include "g722.h"
#include "g722_plc.h"
#include "lpctool.h"
#if (WMOPS)
#include "count.h"
#endif

/*****************************/
#ifdef DYN_RAM_CNT
#include "dyn_ram_cnt.h"
#endif
/*****************************/

/**********************************
* declaration of PLC subroutines *
**********************************/

/* lower-band analysis (main subroutine: G722PLC_ana) */
static void  G722PLC_ana(G722PLC_STATE * plc_state, g722_state *decoder);
static Word16  G722PLC_pitch_ol(Word16 * signal, Word16 *maxco);
static Word16  G722PLC_classif_modif(Word16 maxco, Word16 nbl, Word16 nbh, Word16* mem_speech, Word16 l_mem_speech,
                                     Word16* mem_exc, Word16* t0
                                     );
static void    G722PLC_autocorr(Word16 * x, Word16 * R_h, Word16 * R_l, Word16 ord, Word16 len);
static void    G722PLC_lpc(G722PLC_STATE * plc_state, Word16 * mem_speech); /* interface modified for ONLY_LTP_DC_REMOVE */
static void    G722PLC_residu(G722PLC_STATE * plc_state);


/* lower-band synthesis (main subroutine: G722PLC_syn) */
static void  G722PLC_syn(G722PLC_STATE * plc_state, Word16 * syn, Word16 NumSamples);
static Word16  G722PLC_ltp_pred_1s(Word16* exc, Word16 t0, Word16 *jitter);
static void    G722PLC_ltp_syn(G722PLC_STATE* plc_state, Word16* cur_exc, Word16* cur_syn, Word16 n, Word16 *jitter);
static void    G722PLC_syn_filt(Word16 m, Word16* a, Word16* x, Word16* y, Word16 n);
static void  G722PLC_attenuate(G722PLC_STATE * plc_state, Word16 * cur_sig, Word16 * tabout, Word16 NumSamples, 
                               Word16 * ind, Word16 * weight);
static void  G722PLC_attenuate_lin(G722PLC_STATE * plc_state, Word16 fact, Word16 * cur_sig, Word16 * tabout, Word16 NumSamples, 
                                   Word16 * ind, Word16 * weight);
static void    G722PLC_calc_weight(Word16 *ind_weight, Word16 fact1, Word16 fact2p, Word16 fact3p, Word16 * weight);
static void    G722PLC_update_mem_exc(G722PLC_STATE * plc_state, Word16 * cur_sig, Word16 NumSamples);


/* higher-band synthesis */
static Word16* G722PLC_syn_hb(G722PLC_STATE * plc_state);

static void G722PLC_qmf_updstat ARGS((short *outcode, g722_state *decoder,
                                     short *lb_signal, short *hb_signal, void *plc_state));

void set_att(G722PLC_STATE * plc_state, Word16 inc_att_v, Word16 fact1_v, Word16 fact2p_v, Word16 fact3p_v)
{
  plc_state->inc_att = inc_att_v;  move16();
  plc_state->fact1 = fact1_v; move16();
  plc_state->fact2p = fact2p_v; move16();
  plc_state->fact3p = fact3p_v; move16();
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
void * G722PLC_init(void)
{
  G722PLC_STATE * plc_state;
  Word16 * w16ptr;

  /* allocate memory for PLC plc_state */
  plc_state = (G722PLC_STATE *)malloc(sizeof(G722PLC_STATE));
  if(plc_state == NULL)
  {
    exit(-1);
  }
  w16ptr = (Word16*)plc_state;
  zero16(sizeof(G722PLC_STATE)/2, w16ptr); /*size of G722PLC_STATE structure in Word16 */

  /* LPC, pitch, signal classification parameters */
  plc_state->a = (Word16 *)calloc(ORD_LPC + 1, sizeof(Word16));
  plc_state->mem_syn = (Word16 *)calloc(ORD_LPC, sizeof(Word16));

  zero16(ORD_LPC, plc_state->mem_syn);
  zero16(ORD_LPCP1, plc_state->a);
  plc_state->clas = G722PLC_WEAKLY_VOICED;  move16(); 

  /* signal buffers */
  plc_state->mem_speech = (Word16 *)calloc(MEMSPEECH_LEN, sizeof(Word16));
  plc_state->mem_speech_hb = (Word16 *)calloc(LEN_HB_MEM, sizeof(Word16)); /*MAXPIT is needed, for complexity reason; LEN_HB_MEM: framelength 20ms*/
  plc_state->mem_exc = (Word16 *)calloc(MAXPIT2P1, sizeof(Word16));

  zero16(MEMSPEECH_LEN, plc_state->mem_speech);
  zero16(LEN_HB_MEM, plc_state->mem_speech_hb);
  zero16(MAXPIT2P1, plc_state->mem_exc);

  /* cross-fading */
  plc_state->count_crossfade = CROSSFADELEN; move16();

  /* higher-band hig-pass filtering */
  /* adaptive muting */
  plc_state->weight_lb = 32767;  move16();
  plc_state->weight_hb = 32767;  move16();
  plc_state->inc_att = 1;  move16();
  plc_state->fact1 = FACT1_V;  move16();
  plc_state->fact2p = FACT2P_V;  move16();
  plc_state->fact3p = FACT3P_V;  move16();

#ifdef DYN_RAM_CNT
  {
    UWord32 ssize = 0;
#ifdef MEM_STT
    ssize += (UWord32) (sizeof(G722PLC_STATE));
    ssize += (UWord32) ((ORD_LPC + 1 + ORD_LPC + MEMSPEECH_LEN + LEN_HB_MEM + MAXPIT2P1) * SIZE_Word16);
#endif
    DYN_RAM_PUSH(ssize, "dummy");
  }
#endif

  return((void *)plc_state);
}


/*----------------------------------------------------------------------
* G722PLC_conceal(plc_state, xl, xh, outcode, decoder)
* extrapolation of missing frame
*
* plc_state (i/o) : state variables of PLC
* xl  (o) : decoded lower-band
* xh  (o) : decoder higher-band
* outcode (o) : decoded synthesis
* decoder (i/o) : g722 states (QMF, ADPCM)
*---------------------------------------------------------------------- */
void G722PLC_conceal(void * state, Word16* outcode, g722_state *decoder)
{
  G722PLC_STATE * plc_state = (G722PLC_STATE *) state;
  Word16 Temp, i;
  Word16 * xl, * xh;

#ifdef DYN_RAM_CNT
  {
    UWord32 ssize = 0;
    ssize += (UWord32) ((3) * SIZE_Ptr);
    ssize += (UWord32) ((2) * SIZE_Word16);
    ssize += (UWord32) ((0) * SIZE_Word32);

    DYN_RAM_PUSH(ssize, "dummy");
  }
#endif
  /***********************
  * reset counter *
  ***********************/

  plc_state->count_crossfade = 0;  /* reset counter for cross-fading */
  move16();

  /***********************
  * generate lower band *
  ***********************/

  /* check if first missing frame (i.e. if previous frame received)
  first missing frame -> analyze past buffer + PLC 
  otherwise -> PLC
  */
  xl = &plc_state->mem_speech[257]; /*257 : MEMSPEECH_LEN, L_FRAME_NB*/
  IF(plc_state->prev_bfi == 0)
  {
    plc_state->count_att = 0; move16();  /* reset counter for attenuation in lower band */
    plc_state->count_att_hb = 0;  move16();/* reset counter for attenuation in higher band */
    plc_state->weight_lb = 32767;  move16();
    plc_state->weight_hb = 32767; move16();

    /**********************************
    * analyze buffer of past samples *
    * - LPC analysis
    * - pitch estimation
    * - signal classification
    **********************************/

    G722PLC_ana(plc_state, decoder);

    /******************************
    * synthesize missing samples *
    ******************************/

    /* set increment for attenuation */
    IF(sub(plc_state->clas,G722PLC_VUV_TRANSITION) == 0)
    {
      /* attenuation in 30 ms */
      set_att(plc_state, 2, FACT1_UV, FACT2P_UV, FACT3P_UV);
      Temp = FACT3_UV; move16();
    }
    ELSE
    {
      set_att(plc_state, 1, FACT1_V, FACT2P_V, FACT3P_V);
      Temp = FACT2_V; move16();
    }

    IF(sub(plc_state->clas, G722PLC_TRANSIENT) == 0)
    {
      /* attenuation in 10 ms */
      set_att(plc_state, 6, FACT1_V_R, FACT2P_V_R, FACT3P_V_R);
      Temp = 0; move16();
    }

    /* synthesize lost frame, high band */
    xh = G722PLC_syn_hb(plc_state);

    /*shift low band*/
    mov16(257, &plc_state->mem_speech[L_FRAME_NB], plc_state->mem_speech); /*shift low band*/

    /* synthesize lost frame, low band directly to plc_state->mem_speech*/
    G722PLC_syn(plc_state, xl, L_FRAME_NB);
    FOR(i = 1; i <= 8; i++)
    {
      plc_state->a[i] = mult_r(plc_state->a[i],G722PLC_gamma_az[i]);   move16();
    }
    /* synthesize cross-fade buffer (part of future frame)*/
    G722PLC_syn(plc_state, plc_state->crossfade_buf, CROSSFADELEN);

    /* attenuate outputs */
    G722PLC_attenuate_lin(plc_state, plc_state->fact1, xl, xl, L_FRAME_NB, &plc_state->count_att, &plc_state->weight_lb);
    if(sub(plc_state->clas, G722PLC_TRANSIENT) == 0)
    {
      plc_state->weight_lb = 0;
      move16();
    }
    /*5 ms frame, xfadebuff in 2 parts*/
    G722PLC_attenuate_lin(plc_state, plc_state->fact1, plc_state->crossfade_buf, plc_state->crossfade_buf, CROSSFADELEN/2, &plc_state->count_att, &plc_state->weight_lb);
    G722PLC_attenuate_lin(plc_state, Temp, plc_state->crossfade_buf+L_FRAME_NB, plc_state->crossfade_buf+L_FRAME_NB, CROSSFADELEN/2, &plc_state->count_att, &plc_state->weight_lb);
    G722PLC_attenuate_lin(plc_state, plc_state->fact1, xh, xh, L_FRAME_NB, &plc_state->count_att_hb, &plc_state->weight_hb);
  }
  ELSE
  {
    mov16(257, &plc_state->mem_speech[L_FRAME_NB], plc_state->mem_speech); /*shift*/
    /* copy samples from cross-fading buffer (already generated in previous bad frame decoding)  */

    mov16(L_FRAME_NB, plc_state->crossfade_buf, xl);
    mov16(L_FRAME_NB, &plc_state->crossfade_buf[L_FRAME_NB], plc_state->crossfade_buf); /*shift*/

    /* synthesize 2nd part of cross-fade buffer (part of future frame) and attenuate output */
    G722PLC_syn(plc_state, plc_state->crossfade_buf+L_FRAME_NB, L_FRAME_NB);
    G722PLC_attenuate(plc_state, plc_state->crossfade_buf+L_FRAME_NB, plc_state->crossfade_buf+L_FRAME_NB, L_FRAME_NB, &plc_state->count_att, &plc_state->weight_lb);
    xh = G722PLC_syn_hb(plc_state);
    G722PLC_attenuate(plc_state, xh, xh, L_FRAME_NB, &plc_state->count_att_hb, &plc_state->weight_hb);
  }

  /************************
  * generate higher band *
  ************************/



  /*****************************************
  * QMF synthesis filter and plc_state update *
  *****************************************/

  G722PLC_qmf_updstat(outcode, decoder, xl, xh, plc_state);

  /*****************************/
#ifdef DYN_RAM_CNT
  DYN_RAM_POP();
#endif
  /*****************************/
  return;
}

/*----------------------------------------------------------------------
* G722PLC_clear(plc_state)
* free memory and clear PLC plc_state variables
*
* plc_state (i) : PLC state variables
*---------------------------------------------------------------------- */
void G722PLC_clear(void * state)
{
  G722PLC_STATE * plc_state = (G722PLC_STATE *) state;

  free(plc_state->mem_speech);
  free(plc_state->mem_speech_hb);
  free(plc_state->mem_exc);
  free(plc_state->a);
  free(plc_state->mem_syn);
  free(plc_state);


  /*****************************/
#ifdef DYN_RAM_CNT
  DYN_RAM_POP();
#endif
  /*****************************/
}


/*********************************
* definition of PLC subroutines *
*********************************/

/*----------------------------------------------------------------------
* G722PLC_hp50(x1, y1_lo, y2_hi, signal)
* 50 Hz high-pass filter
*
* x1          (i/o) : filter memory
* y1_hi,y1_lo (i/o) : filter memory
* signal     (i)   : input sample
*----------------------------------------------------------------------*/

Word16 G722PLC_hp(Word16 *x1, Word16* y1_hi, Word16 *y1_lo, Word16 signal, 
                  const Word16 *G722PLC_b_hp, const Word16 *G722PLC_a_hp)
{
  Word32    ACC0, ACC1;

#ifdef DYN_RAM_CNT
  {
    UWord32 ssize = 0;
    ssize += (UWord32) ((0) * SIZE_Ptr);
    ssize += (UWord32) ((0) * SIZE_Word16);
    ssize += (UWord32) ((2) * SIZE_Word32);

    DYN_RAM_PUSH(ssize, "dummy");
  }
#endif
  /*  y[i] =      x[i]   -         x[i-1]    */
  /*                     + 123/128*y[i-1]    */
  ACC0 = L_mult(signal, G722PLC_b_hp[0]);
  ACC0 = L_mac(ACC0, *x1, G722PLC_b_hp[1]);
  *x1 = signal;
  move16();

  ACC0 = L_mac(ACC0, *y1_hi, G722PLC_a_hp[1]);
  ACC1 = L_mult(*y1_lo, G722PLC_a_hp[1]);

  /*    ACC0 = L_shl(ACC0, 2); */ /* Q29 --> Q31  */
  ACC0 = L_add(ACC0, L_shr(ACC1, 15));

  L_Extract(ACC0, y1_hi, y1_lo);

  /*****************************/
#ifdef DYN_RAM_CNT
  DYN_RAM_POP();
#endif
  /*****************************/
  return(round_fx(ACC0));
}


/*----------------------------------------------------------------------
* G722PLC_syn_hb(plc_state, xh, n)
* reconstruct higher-band by pitch prediction
*
* plc_state (i/o) : plc_state variables of PLC
*---------------------------------------------------------------------- */

static Word16* G722PLC_syn_hb(G722PLC_STATE* plc_state)
{
  Word16 *ptr;
  Word16 *ptr2;
  Word16 loc_t0;
  Word16   tmp;

#ifdef DYN_RAM_CNT
  {
    UWord32 ssize = 0;
    ssize += (UWord32) ((2) * SIZE_Ptr);
    ssize += (UWord32) ((2) * SIZE_Word16);
    ssize += (UWord32) ((0) * SIZE_Word32);

    DYN_RAM_PUSH(ssize, "dummy");
  }
#endif
  /* save pitch delay */
  loc_t0 = plc_state->t0;
  move16();

  /* if signal is not voiced, cut harmonic structure by forcing a 10 ms pitch */
  if(plc_state->clas != 0) /*constant G722PLC_VOICED = 0*/
  {
    loc_t0 = 80;
    move16();
  }

  IF(sub(plc_state->clas,1) == 0)/*G722PLC_UNVOICED*/
  {
    Word32 mean;
    Word16 smean, tmp1, i;
#ifdef DYN_RAM_CNT
    {
      UWord32 ssize = 0;
      ssize += (UWord32) ((0) * SIZE_Ptr);
      ssize += (UWord32) ((3) * SIZE_Word16);
      ssize += (UWord32) ((1) * SIZE_Word32);

      DYN_RAM_PUSH(ssize, "dummy");
    }
#endif

    mean = 0;
    move32();
    tmp1 = sub(LEN_HB_MEM, 80); /* tmp1 = start index of last 10 ms, last periode is smoothed */
    FOR(i = 0; i < 80; i++)
    {
      mean = L_mac0(mean, abs_s(plc_state->mem_speech_hb[tmp1 + i]), 1);
    }
    mean = L_shr(mean, 5);  /*80/32 = 2.5 mean amplitude*/
    smean = extract_l(mean);

    tmp1 = sub(LEN_HB_MEM, loc_t0); /* tmp1 = start index of last periode that is smoothed */
    FOR(i = 0; i < loc_t0; i++)
    {
      if(sub(abs_s(plc_state->mem_speech_hb[tmp1 + i]), smean) > 0)
      {
        plc_state->mem_speech_hb[tmp1 + i] = shr(plc_state->mem_speech_hb[tmp1 + i], 2);
        move16();
      }
    }
    /*****************************/
#ifdef DYN_RAM_CNT
    DYN_RAM_POP();
#endif
    /*****************************/
  }

  /* reconstruct higher band signal by pitch prediction */
  tmp = sub(L_FRAME_NB,loc_t0);
  ptr = plc_state->mem_speech_hb + sub(LEN_HB_MEM, loc_t0); /*beginning of copy zone*/
  ptr2 = plc_state->mem_speech_hb + LEN_HB_MEM_MLF; /*beginning of last frame in mem_speech_hb*/
  IF(tmp <= 0) /* l_frame <= t0*/
  {
    /* temporary save of new frame in plc_state->mem_speech[0 ...L_FRAME_NB-1] of low_band!! that will be shifted after*/
    mov16(L_FRAME_NB, ptr, plc_state->mem_speech);
    mov16(LEN_HB_MEM_MLF, &plc_state->mem_speech_hb[L_FRAME_NB], plc_state->mem_speech_hb); /*shift 1 frame*/

    mov16(L_FRAME_NB, plc_state->mem_speech, ptr2);
  }
  ELSE /*t0 < L_FRAME_NB*/
  {
    mov16(LEN_HB_MEM_MLF, &plc_state->mem_speech_hb[L_FRAME_NB], plc_state->mem_speech_hb); /*shift memory*/

    mov16(loc_t0, ptr, ptr2); /*copy last period*/
    mov16(tmp, ptr2, &ptr2[loc_t0]); /*repeate last period*/
  }
  /*****************************/
#ifdef DYN_RAM_CNT
  DYN_RAM_POP();
#endif
  /*****************************/
  return(ptr2);
}


/*-------------------------------------------------------------------------*
* G722PLC_attenuate(state, in, out, n, count, weight)
* linear muting with adaptive slope
*
* state (i/o) : PLC state variables
* in    (i)   : input signal
* out (o)   : output signal = attenuated input signal
* n   (i)   : number of samples
* count (i/o) : counter
* weight (i/o): muting factor
*--------------------------------------------------------------------------*/
static void G722PLC_attenuate(G722PLC_STATE* plc_state, Word16* in, Word16* out, Word16 n, Word16 *count, Word16 * weight)
{
  Word16 i;
#ifdef DYN_RAM_CNT
  {
    UWord32 ssize = 0;
    ssize += (UWord32) ((0) * SIZE_Ptr);
    ssize += (UWord32) ((1) * SIZE_Word16);
    ssize += (UWord32) ((0) * SIZE_Word32);

    DYN_RAM_PUSH(ssize, "dummy");
  }
#endif

  FOR (i = 0; i < n; i++)
  {
    /* calculate attenuation factor and multiply */
    G722PLC_calc_weight(count, plc_state->fact1, plc_state->fact2p, plc_state->fact3p, weight);
    out[i] = mult_r(*weight, in[i]);
    move16();
    *count = add(*count, plc_state->inc_att);
  }
  /*****************************/
#ifdef DYN_RAM_CNT
  DYN_RAM_POP();
#endif
  /*****************************/
  return;
}

/*-------------------------------------------------------------------------*
* G722PLC_attenuate_lin(plc_state, fact, in, out, n, count, weight)
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

static void G722PLC_attenuate_lin(G722PLC_STATE* plc_state, Word16 fact, Word16* in, Word16* out, Word16 n, Word16 *count, Word16 * weight)
{
  Word16 i;

#ifdef DYN_RAM_CNT
  {
    UWord32 ssize = 0;
    ssize += (UWord32) ((0) * SIZE_Ptr);
    ssize += (UWord32) ((1) * SIZE_Word16);
    ssize += (UWord32) ((0) * SIZE_Word32);

    DYN_RAM_PUSH(ssize, "dummy");
  }
#endif
  FOR (i = 0; i < n; i++) /*adaptation 5ms*/
  {
    /* calculate attenuation factor and multiply */
    *weight = sub(*weight, fact);
    out[i] = mult_r(*weight, in[i]);
    move16();
  }
  *count = add(*count, i_mult(plc_state->inc_att, n)); /*adaptation 5ms*/

  /*****************************/
#ifdef DYN_RAM_CNT
  DYN_RAM_POP();
#endif
  /*****************************/
  return;
}

/*-------------------------------------------------------------------------*
* G722PLC_calc_weight(ind_weight, Ind1, Ind12, Ind123, Rlim1, Rlim2, fact1, fact2, fact3,
*                     tab_len, WeightEnerSynthMem)
* calculate attenuation factor
*--------------------------------------------------------------------------*/

static void G722PLC_calc_weight(Word16 *ind_weight, Word16 fact1, Word16 fact2p, Word16 fact3p, Word16 * weight)
{

  *weight = sub(*weight, fact1);
  if (sub(*ind_weight, END_1ST_PART) >= 0)
  {
    *weight = sub(*weight, fact2p);
  }
  if (sub(*ind_weight, END_2ND_PART) >= 0)
  {
    *weight = sub(*weight, fact3p);
  }
  if (sub(*ind_weight, END_3RD_PART) >= 0)
  {
    *weight = 0;   move16();
  }
  if(*weight <= 0)
  {
    *ind_weight = END_3RD_PART;   move16();
  }
  return;
}

/*-------------------------------------------------------------------------*
* Function G722PLC_update_mem_exc                                          *
* Update of plc_state->mem_exc and shifts the memory                           *
* if plc_state->t0 > L_FRAME_NB                                            *
*--------------------------------------------------------------------------*/
static void G722PLC_update_mem_exc(G722PLC_STATE * plc_state, Word16 * exc, Word16 n)
{
  Word16 *ptr;
  Word16 temp;

#ifdef DYN_RAM_CNT
  {
    UWord32 ssize = 0;
    ssize += (UWord32) ((1) * SIZE_Ptr);
    ssize += (UWord32) ((1) * SIZE_Word16);
    ssize += (UWord32) ((0) * SIZE_Word32);

    DYN_RAM_PUSH(ssize, "dummy");
  }
#endif
  /* shift ResMem, if t0 > l_frame */
  temp = sub(plc_state->t0p2, n);

  ptr = plc_state->mem_exc + sub(MAXPIT2P1, plc_state->t0p2);
  IF (temp > 0)
  {
    mov16(temp, &ptr[n], ptr);
    mov16(n, exc, &ptr[temp]);
  }
  ELSE
  {
    /* copy last "pitch cycle" of residual */
    mov16(plc_state->t0p2, &exc[sub(n, plc_state->t0p2)], ptr);
  }

  /*****************************/
#ifdef DYN_RAM_CNT
  DYN_RAM_POP();
#endif
  /*****************************/
  return;
}

/*-------------------------------------------------------------------------*
* Function G722PLC_ana(plc_state, decoder)                   *
* Main analysis routine
*
* plc_state   (i/o) : PLC state variables
* decoder (i)   : G.722 decoder state variables
*-------------------------------------------------------------------------*/

static void G722PLC_ana(G722PLC_STATE * plc_state, g722_state *decoder)
{  
  Word16 maxco;
  Word16 nooffsig[MEMSPEECH_LEN];
  Word16 i;

  Word16 x1, y1_hi, y1_lo;

#ifdef DYN_RAM_CNT
  {
    UWord32 ssize = 0;
    ssize += (UWord32) ((0) * SIZE_Ptr);
    ssize += (UWord32) ((MEMSPEECH_LEN + 5) * SIZE_Word16);
    ssize += (UWord32) ((0) * SIZE_Word32);

    DYN_RAM_PUSH(ssize, "dummy");
  }
#endif

  /* DC-remove filter */
  x1 = y1_hi = y1_lo = 0;
  move16();
  move16();
  move16(); 
  /* DC-remove filter */
  FOR(i = 0; i < MEMSPEECH_LEN; i++)
  {
    nooffsig[i] = G722PLC_hp(&x1, &y1_hi, &y1_lo, plc_state->mem_speech[i],
      G722PLC_b_hp, G722PLC_a_hp);
    move16(); 
  }

  /* perform LPC analysis and compute residual signal */
  G722PLC_lpc(plc_state, nooffsig);
  G722PLC_residu(plc_state);
  /* estimate (open-loop) pitch */
  /* attention, may shift noofsig, but only used after for zero crossing rate not influenced by this shift (except for very small values)*/
  plc_state->t0 = G722PLC_pitch_ol(nooffsig + MEMSPEECH_LEN - MAXPIT2,
    &maxco);

  /* update memory for LPC
  during ereased period the plc_state->mem_syn contains the non weighted
  synthetised speech memory. For thefirst erased frame, it
  should contain the output speech.
  Saves the last ORD_LPC samples of the output signal in
  plc_state->mem_syn    */
  mov16(ORD_LPC, &plc_state->mem_speech[MEMSPEECH_LEN - ORD_LPC],
    plc_state->mem_syn);

  /* determine signal classification and modify residual in case of transient */
  plc_state->clas = G722PLC_classif_modif(maxco, decoder->nbl, decoder->nbh, nooffsig, MEMSPEECH_LEN,
    plc_state->mem_exc, &plc_state->t0

    );

  plc_state->t0p2 = add(plc_state->t0,2); move16();

  /*****************************/
#ifdef DYN_RAM_CNT
  DYN_RAM_POP();
#endif
  /*****************************/

  return;
}

/*-------------------------------------------------------------------------*
* Function G722PLC_autocorr*
*--------------------------------------------------------------------------*/

void G722PLC_autocorr(Word16 x[],  /* (i)    : Input signal                      */
                      Word16 r_h[],/* (o)    : Autocorrelations  (msb)           */
                      Word16 r_l[], /* (o)    : Autocorrelations  (lsb)           */
                      Word16 ord,    /* (i)    : LPC order                         */
                      Word16 len /* (i)    : length of analysis                   */
                      )
{
  Word32 sum;
  Word16 i, norm;
  Word16 *y = (Word16 *)calloc(len, sizeof(Word16));
#ifdef DYN_RAM_CNT
  {
    UWord32 ssize = 0;
    ssize += (UWord32) ((1) * SIZE_Ptr);
    ssize += (UWord32) ((len + 2) * SIZE_Word16);
    ssize += (UWord32) ((1) * SIZE_Word32);

    DYN_RAM_PUSH(ssize, "dummy");
  }
#endif

  /* Windowing of signal */
  FOR(i = 0; i < len; i++)
  {
    y[i] = mult_r(x[i], G722PLC_lpc_win_80[HAMWINDLEN-len+i]); /* for length < 80, uses the end of the window */
    move16();
  }
  /* Compute r[0] and test for overflow */

  DO
  {
    Overflow = 0;
    move16();
    sum = L_add(1, L_mac_Array(len, y, y));

    /* If overflow divide y[] by 4 */

    IF(Overflow != 0)
    {
      array_oper(len, 2, y, y, &shr);

    }
  }
  WHILE(Overflow != 0);

  /* Normalization of r[0] */
  sum = norm_l_L_shl(&norm, sum);
  L_Extract(sum, &r_h[0], &r_l[0]); /* Put in DPF format (see oper_32b) */

  /* r[1] to r[m] */

  FOR(i = 1; i <= ord; i++)
  {
    sum = L_mac_Array(sub(len, i), y, &y[i]);
    sum = L_shl(sum, norm);
    L_Extract(sum, &r_h[i], &r_l[i]);
  }
  free(y);
  /*****************************/
#ifdef DYN_RAM_CNT
  DYN_RAM_POP();
#endif
  /*****************************/

  return;
}

static void G722PLC_pitch_ol_refine(Word16 * nooffsigptr, Word16 il, Word32 ener1_f, Word16 ne1, 
                                    Word16 beg_last_per, Word16 end_last_per,
                                    Word16 *ind, Word16 *maxco)
{
  Word16 i, j; 
  Word32 corx_f, ener2_f;
  Word16 start_ind, end_ind;
  Word16 e1, e2, co, em, norm_e;
  Word32 ener1n, ener2n;

#ifdef DYN_RAM_CNT
  {
    UWord32 ssize = 0;
    ssize += (UWord32) ((0) * SIZE_Ptr);
    ssize += (UWord32) ((9) * SIZE_Word16);
    ssize += (UWord32) ((4) * SIZE_Word32);

    DYN_RAM_PUSH(ssize, "dummy");
  }
#endif

  start_ind = sub(il, 2);
  start_ind = s_max(start_ind, MINPIT);
  end_ind = add(il, 2);
  j = sub(end_last_per, start_ind);
  ener2_f = L_mac0(1, nooffsigptr[j], nooffsigptr[j]); /*to avoid division by 0*/
  FOR(j = sub(end_last_per, 1); j > beg_last_per; j--)
  {
    ener2_f= L_mac0(ener2_f, nooffsigptr[j-start_ind], nooffsigptr[j-start_ind]);
  }
  FOR(i = start_ind; i <= end_ind; i++)
  {
    corx_f = 0;   move32();

    ener2_f = L_mac0(ener2_f, nooffsigptr[beg_last_per-i], nooffsigptr[beg_last_per-i]); /*update, part 2*/
    FOR(j = end_last_per; j >= beg_last_per; j--)
    {
      corx_f= L_mac0(corx_f, nooffsigptr[j], nooffsigptr[j-i]);
    }
    norm_e = s_min(ne1, norm_l(ener2_f));
    ener1n = L_shl(ener1_f, norm_e);
    ener2n = L_shl(ener2_f, norm_e);
    corx_f = L_shl(corx_f, norm_e);
    e1 = round_fx(ener1n);
    e2 = round_fx(ener2n);
    co = round_fx(corx_f);
    em = s_max(e1, e2);
    em = s_max(co, em);

    if(co > 0)
    {
      co = div_s(co, em);
    }

    if(sub(co, *maxco) > 0)
    {
      *ind = i;   move16();
    }
    *maxco = s_max(co, *maxco); move16();
    ener2_f = L_msu0(ener2_f, nooffsigptr[end_last_per-i], nooffsigptr[end_last_per-i]); /*update, part 1*/
  }
  /*****************************/
#ifdef DYN_RAM_CNT
  DYN_RAM_POP();
#endif
  /*****************************/
  return;
}

/*----------------------------------------------------------------------
* G722PLC_pitch_ol(signal, length, maxco)
* open-loop pitch estimation
*
* signal      (i) : pointer to signal buffer (including signal memory)
* length      (i) : length of signal memory
* maxco       (o) : maximal correlation
* overlf_shft (o) : number of shifts
*
*---------------------------------------------------------------------- */

static Word16 G722PLC_pitch_ol(Word16 * signal, Word16 *maxco)
{  
  Word16 i, j, il, k; 
  Word16 ind, ind2;
  Word16 *w_ds_sig;
  Word32 corx_f, ener1_f, ener2_f;
  Word32 temp_f;
  Word16 valid = 0; /*not valid for the first lobe */
  Word16 beg_last_per;
  Word16 e1, e2, co, em, norm_e, ne1;
  Word32 ener1n, ener2n;
  Word16 *ptr1, *nooffsigptr;
  Word32 L_temp; 
  Word16 maxco_s8, stable;
  Word16 previous_best, overfl_shft;

  Word16 ds_sig[MAXPIT2_DS];
  Word16 ai[3], cor_h[3], cor_l[3], rc[3];
  Word16 *pt1, *pt2;
  Word16 zcr;
  Word16 *w_ds_sig_alloc;

  move16(); 

#ifdef DYN_RAM_CNT
  {
    UWord32 ssize = 0;
    ssize += (UWord32) ((6) * SIZE_Ptr);
    ssize += (UWord32) ((19 + MAXPIT2_DS + 4*3) * SIZE_Word16);
    ssize += (UWord32) ((7) * SIZE_Word32);

    DYN_RAM_PUSH(ssize, "dummy");
  }
#endif

  nooffsigptr = signal; /* 296 - 8; rest MAXPIT2 */

  /* downsample (filter and decimate) signal */
  ptr1 = ds_sig;
  FOR(i = FACT_M1; i < MAXPIT2; i += FACT)
  {
    temp_f = L_mult0(nooffsigptr[i], G722PLC_fir_lp[0]);
    pt2 = nooffsigptr+i;
    FOR (k = 1; k < FEC_L_FIR_FILTER_LTP; k++)
    {
      pt2--;
      temp_f = L_mac0(temp_f, *pt2, G722PLC_fir_lp[k]);
    }
    *ptr1++ = round_fx(temp_f);
    move16();
  }

  G722PLC_autocorr(ds_sig, cor_h, cor_l, 2, MAXPIT2_DS);
  Lag_window(cor_h, cor_l, G722PLC_lag_h, G722PLC_lag_l, 2);/* Lag windowing*/
  Levinson(cor_h, cor_l, rc, &stable, 2, ai);
  ai[1] = mult_r(ai[1],GAMMA);
  ai[2] = mult_r(ai[2],GAMMA2);

  move16();
  move16();

  /* filter */
  w_ds_sig_alloc = (Word16 *)calloc(MAXPIT2_DS-2, sizeof(Word16));
  w_ds_sig = w_ds_sig_alloc-2;
#ifdef DYN_RAM_CNT
  {
    UWord32 ssize = 0;
    ssize += (UWord32) ((0) * SIZE_Ptr);
    ssize += (UWord32) ((MAXPIT2_DS-2) * SIZE_Word16);
    ssize += (UWord32) ((0) * SIZE_Word32);

    DYN_RAM_PUSH(ssize, "dummy");
  }
#endif

  FOR (i = 2; i < MAXPIT2_DS; i++)
  {
    L_temp = L_mult(ai[1], ds_sig[i - 1]);   
    L_temp = L_mac(L_temp, ai[2], ds_sig[i - 2]);   
    w_ds_sig[i] = add(ds_sig[i], round_fx_L_shl(L_temp,3));   move16();
  }

  ind = MAXPIT_S2_DS; move16(); /*default value, 18*/
  previous_best = 0;  move16(); /*default value*/
  ind2 = 1;  move16();

  /*Test overflow on w_ds_sig*/
  overfl_shft = 0;   move16();

  /* compute energy of signal in range [len/fac-1,(len-MAX_PIT)/fac-1] */
  ener1_f = L_add(1, L_mac0_Array(MAXPIT2_DSM1-MAXPIT_DSP1+1, &w_ds_sig[MAXPIT_DSP1], &w_ds_sig[MAXPIT_DSP1]));

  /* compute exponent */
  ne1 = norm_l(ener1_f);

  /* compute maximal correlation (maxco) and pitch lag (ind) */
  *maxco = 0;  move16();
  ener2_f = L_msu0(ener1_f, w_ds_sig[MAXPIT2_DSM1], w_ds_sig[MAXPIT2_DSM1]); /*update, part 1*/
  ener2_f = L_mac0(ener2_f, w_ds_sig[MAXPIT_DSP1-1], w_ds_sig[MAXPIT_DSP1-1]); /*update, part 2*/
  ener2_f = L_msu0(ener2_f, w_ds_sig[MAXPIT2_DSM1-1], w_ds_sig[MAXPIT2_DSM1-1]); /*update, part 1*/
  pt1 = &w_ds_sig[MAXPIT2_DSM1];
  pt2 = pt1-1;   
  zcr = 0;  move16();
  FOR(i = 2; i < MAXPIT_DS; i++) /* < to avoid out of range later*/
  {
    ind2 = add(ind2, 1);
    corx_f = 0;    move32();

    FOR(j = MAXPIT2_DSM1; j >= MAXPIT_DSP1; j--)
    {
      corx_f = L_mac0(corx_f, w_ds_sig[j], w_ds_sig[j-i]);
    }
    ener2_f = L_mac0(ener2_f, w_ds_sig[MAXPIT_DSP1-i], w_ds_sig[MAXPIT_DSP1-i]); /*update, part 2*/
    norm_e = s_min(ne1, norm_l(ener2_f));
    ener1n = L_shl(ener1_f, norm_e);
    ener2n = L_shl(ener2_f, norm_e);
    corx_f = L_shl(corx_f, norm_e);
    e1 = round_fx(ener1n);
    e2 = round_fx(ener2n);
    ener2_f = L_msu0(ener2_f, w_ds_sig[MAXPIT2_DSM1-i], w_ds_sig[MAXPIT2_DSM1-i]); /*update, part 1*/
    co = round_fx(corx_f);
    em = s_max(e1, e2);
    em = s_max(co, em);

    if(co > 0)
    {
      co = div_s(co, em); /*normalized cross-correlation*/
    }

    if(co < 0) 
    {
      valid = 1;   move16();
    }
    /* compute (update)zero-crossing  in last examined period */
    if(s_and(s_xor(*pt1, *pt2),(Word16)0x8000) != 0)
    {
      zcr = add(zcr, 1);
    }
    pt1--;
    pt2--;
    if(zcr == 0) /*no zero crossing*/
    {
      valid = 0;     move16();
    }

    IF(valid > 0)
    {
      test();
      IF((sub(ind2, ind) == 0) || (sub(ind2, shl(ind,1)) == 0))
      {
        if(sub(*maxco, 27850) > 0) /* 0.85 : high correlation, small chance that double pitch is OK*/
        {
          *maxco = 32767;   move16();
        }

        maxco_s8 = shr(*maxco, 3);
        if(sub(*maxco, 29126) < 0)/*to avoid overflow*/
        {
          *maxco = add(*maxco, maxco_s8); 
        }
      }

      test();
      IF((sub(co, *maxco) > 0) && (sub(i, MINPIT_DS) >= 0))
      {
        *maxco = co; move16();
        if(sub(i, add(ind,1)) != 0)
        {
          previous_best = ind; move16();
        }
        ind = i; move16();
        ind2 = 1; move16(); 
      }
    }
  }
  free(w_ds_sig_alloc);

  /*****************************/
#ifdef DYN_RAM_CNT
  DYN_RAM_POP();
#endif
  /*****************************/

  /* convert pitch to non decimated domain */
  il = shl(ind, FACTLOG2);
  ind = il;  move16();

  /* shift DC-removed signal to avoid overflow in correlation */
  if(L_sub(ener1_f, 0x01000000) > 0) /* maxcor will be computed on 4* points in non weighted domain --> overflow risq*/
  {
    overfl_shft = add(overfl_shft,1);
  }

  IF(overfl_shft > 0)
  {
    array_oper(MAXPIT2-1, overfl_shft, &nooffsigptr[1], &nooffsigptr[1], &shr);
  }

  /* refine pitch in non-decimated (8 kHz) domain by step of 1
  -> maximize correlation around estimated pitch lag (ind) */
  beg_last_per = sub(MAXPIT2, il);
  ener1_f = L_mac0(1, nooffsigptr[END_LAST_PER], nooffsigptr[END_LAST_PER]); /*to avoid division by 0*/
  FOR(j = END_LAST_PER_1; j >= beg_last_per; j--)
  {
    ener1_f= L_mac0(ener1_f, nooffsigptr[j], nooffsigptr[j]);
  }
  /* compute exponent */
  ne1 = norm_l(ener1_f);
  /* compute maximal correlation (maxco) and pitch lag (ind) */
  *maxco = 0;  move16();

  G722PLC_pitch_ol_refine(nooffsigptr, il, ener1_f, ne1, beg_last_per, END_LAST_PER, &ind,  maxco);
  if(sub(*maxco, 13107) < 0) /*check 2nd candidate if maxco > 0.4*/
  {
    previous_best = 0;    move16();
  }
  IF(previous_best != 0) /*check second candidate*/
  {
    il = shl(previous_best, FACTLOG2);
    G722PLC_pitch_ol_refine(nooffsigptr, il, ener1_f, ne1, beg_last_per, END_LAST_PER, &ind,  maxco);
  }

  IF (sub(*maxco, 8192) < 0)
  {
    if(sub(ind, 32) < 0)
    {
      ind = shl(ind,1); /*2 times pitch for very small pitch, at least 2 times MINPIT */
    }
  }
  /*****************************/
#ifdef DYN_RAM_CNT
  DYN_RAM_POP();
#endif
  /*****************************/

  return ind;
}

/*----------------------------------------------------------------------
* G722PLC_classif_modif(maxco, decoder)
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

static Word16 G722PLC_classif_modif(Word16 maxco, Word16 nbl, Word16 nbh, Word16* mem_speech, Word16 l_mem_speech,
                                    Word16* mem_exc, Word16* t0
                                    )
{
  Word16 clas, Temp, tmp1, tmp2, tmp3, tmp4, i, maxres, absres, zcr, clas2;
  Word16 *pt1, *pt2;
#ifdef DYN_RAM_CNT
  {
    UWord32 ssize = 0;
    ssize += (UWord32) ((2) * SIZE_Ptr);
    ssize += (UWord32) ((11) * SIZE_Word16);
    ssize += (UWord32) ((0) * SIZE_Word32);

    DYN_RAM_PUSH(ssize, "dummy");
  }
#endif

  /************************************************************************
  * select preliminary class => clas = UNVOICED, WEAKLY_VOICED or VOICED *
  * by default clas=WEAKLY_VOICED                         *
  * classification criterio:                                             *
  * -normalized correlation from open-loop pitch                         *
  * -ratio of lower/higher band energy (G722 scale factors)              *
  * -zero crossing rate                                                  *
  ************************************************************************/

  /* compute zero-crossing rate in last 10 ms */
  pt1 = &mem_speech[sub(l_mem_speech, 80)];
  pt2 = pt1-1;   
  zcr = 0;
  move16();
  FOR(i = 0; i< 80; i++)
  {
    Temp = 0;
    move16();
    if(*pt1 <= 0)
    {
      Temp = 1;
      move16();
    }
    if(*pt2 > 0)
    {
      Temp = add(Temp,1);
    }

    zcr = add(zcr, shr(Temp,1));
    pt1++;
    pt2++;
  }

  /* set default clas */
  clas = G722PLC_WEAKLY_VOICED;
  move16();

  /* detect voiced clas (corr > 3/4 ener1 && corr > 3/4 ener2) */
  if(sub(maxco, 22936) > 0) /* 22936 in Q15 = 0.7 */
  {
    clas = G722PLC_VOICED;
    move16();
  }

  /* change class to unvoiced if higher band has lots of energy
  (risk of "dzing" if clas is "very voiced") */
  IF(sub(nbh, nbl) > 0)
  {
    clas2 = clas;
    clas = G722PLC_VUV_TRANSITION;
    move16();
    move16();

    if(clas2 == 0)  /*if the class is VOICED (constant G722PLC_VOICED = 0)*/
    {
      clas = G722PLC_WEAKLY_VOICED;
      move16();
    }
  }

  /* change class to unvoiced if zcr is high */
  IF (sub(zcr,20)>=0)
  {
    clas = G722PLC_UNVOICED;
    move16();
    /* change pitch if unvoiced class (to avoid short pitch lags) */
    if(sub(*t0, 32) < 0)
    {
      *t0 = shl(*t0,1); /*2 times pitch for very small pitch, at least 2 times MINPIT */
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
  IF (sub(clas,4) > 0)/*G722PLC_WEAKLY_VOICED(5) or G722PLC_VUV_TRANSITION(7)*/
  {
    tmp1 = sub(MAXPIT2P1, *t0); /* tmp1 = start index of last "pitch cycle" */
    tmp2 = sub(tmp1, *t0);  /* tmp2 = start index of last but one "pitch cycle" */
    FOR(i = 0; i < *t0; i++)
    {
      tmp3 = add(tmp2, i);

      maxres = s_max(s_max(abs_s(mem_exc[tmp3-2]), abs_s(mem_exc[tmp3-1])),   
        s_max(abs_s(mem_exc[tmp3]),   abs_s(mem_exc[tmp3+1])));   
      maxres = s_max(abs_s(mem_exc[tmp3+2]), maxres);   
      absres = abs_s(mem_exc[tmp1 + i]);   

      /* if magnitude in last cycle > magnitude in last but one cycle */
      IF(sub(absres, maxres) > 0)
      {
        /* detect transient (ratio = 1/8) */
        tmp4 = shr(absres,3);
        if(sub(tmp4, maxres) >= 0) 
        {
          Temp = add(Temp, 1);
        }

        /* limit value (even if a transient is not detected...) */
        if(mem_exc[tmp1 + i] < 0)   
        {
          mem_exc[tmp1 + i] = negate(maxres);   
        }
        if(mem_exc[tmp1 + i] >= 0)   
        {
          mem_exc[tmp1 + i] = maxres;   
          move16();
        }
      }
    }
  }
  IF(sub(clas,1) == 0)/*G722PLC_UNVOICED*/
  {
    Word32 mean;
    Word16 smean;

#ifdef DYN_RAM_CNT
    {
      UWord32 ssize = 0;
      ssize += (UWord32) ((0) * SIZE_Ptr);
      ssize += (UWord32) ((1) * SIZE_Word16);
      ssize += (UWord32) ((1) * SIZE_Word32);

      DYN_RAM_PUSH(ssize, "dummy");
    }
#endif
    mean = 0;

    move32();
    /* 209 = MAXPIT2P1 - 80, start index of last 10 ms, last period is smoothed */
    FOR(i = 0; i < 80; i++)
    {
      mean = L_mac0(mean, abs_s(mem_exc[209 + i]), 1);
    }
    mean = L_shr(mean, 5);  /*80/32 = 2.5 mean amplitude*/
    smean = extract_l(mean);

    tmp1 = sub(MAXPIT2P1, *t0); /* tmp1 = start index of last "pitch cycle" */

    FOR(i = 0; i < *t0; i++)
    {
      if(sub(abs_s(mem_exc[tmp1 + i]), smean) > 0)
      {
        mem_exc[tmp1 + i] = shr(mem_exc[tmp1 + i], 2);   
      }
    }
    /*****************************/
#ifdef DYN_RAM_CNT
    DYN_RAM_POP();
#endif
    /*****************************/
  }
  if (Temp>0)
  {
    clas = G722PLC_TRANSIENT;

    *t0 = s_min(*t0, 40); /*max 5 ms pitch */
    move16();
  }

  /*******************************************************************************
  * pitch tuning by searching last glotal pulses                                *
  * checks if there is no 2 pulses in the last periode due to decreasing pitch  *
  *******************************************************************************/

  IF(clas == 0)  /*if the class is VOICED (constant G722PLC_VOICED = 0)*/
  {
    Word16 maxpulse, pulseind=0; 
    Word16 mincheck;
    Word16 end2nd;
    Word16 maxpulse2nd, pulseind2nd=0; 
    Word16 absval;
    Word32 cumul, pulsecumul;
    Word16 signbit, signbit2nd;
#ifdef DYN_RAM_CNT
    {
      UWord32 ssize = 0;
      ssize += (UWord32) ((0) * SIZE_Ptr);
      ssize += (UWord32) ((9) * SIZE_Word16);
      ssize += (UWord32) ((2) * SIZE_Word32);

      DYN_RAM_PUSH(ssize, "dummy");
    }
#endif
    move16();
    move16();

    mincheck = sub(*t0,5);
    maxpulse = -1;
    maxpulse2nd = -1;
    cumul = 0;
    pulsecumul = 0;
    move16();
    move16();
    move32();
    move32();

    pt1 = &mem_exc[MAXPIT2P1 - 1]; /*check the last period*/

    FOR(i = 0; i < *t0; i++) /*max pitch variation searched is +-5 */
    {
      absval = abs_s(*pt1);
      if(sub(absval, maxpulse) > 0)
      {
        pulseind = i;
        move16();
      }
      maxpulse = s_max(absval, maxpulse);
      cumul = L_mac0(cumul, absval, 1);
      pt1--;
    }
    pulsecumul = L_mult0(maxpulse, *t0);
    signbit = s_and(mem_exc[sub(MAXPIT2P1, add(pulseind, 1))],(Word16)0x8000); /*check the sign*/


    IF(L_sub(cumul, L_shr(pulsecumul,2)) < 0) /* if mean amplitude < max amplitude/4 --> real pulse*/
    {
      end2nd = sub(pulseind, mincheck);
      pt1 = &mem_exc[MAXPIT2P1 - 1]; /*end of excitation*/

      FOR(i = 0; i < end2nd; i++) /*search 2nd pulse at the end of the periode*/
      {
        absval = abs_s(*pt1);  /*abs_s added on 25/07/2007*/
        if(sub(absval, maxpulse2nd) > 0)
        {
          pulseind2nd = i;
          move16();
        }
        maxpulse2nd = s_max(absval, maxpulse2nd);
        pt1--;
      }
      end2nd = add(mincheck,pulseind);
      pt1 = &mem_exc[sub(MAXPIT2P1, add(1, end2nd))]; /*end of excitation*/

      FOR(i = end2nd; i < *t0; i++) /*search 2nd pulse at the beggining of the periode*/
      {
        absval = abs_s(*pt1);
        if(sub(absval, maxpulse2nd) > 0)
        {
          pulseind2nd = i;
          move16();
        }
        maxpulse2nd = s_max(absval, maxpulse2nd);
        pt1--;
      }
      IF(sub(maxpulse2nd, shr(maxpulse,1)) > 0)
      {
        signbit2nd = s_and(mem_exc[sub(MAXPIT2P1, add(pulseind2nd, 1))],(Word16)0x8000); /*check the sign*/

        if(s_xor(signbit, signbit2nd) == 0)
        {
          *t0 = abs_s(sub(pulseind,pulseind2nd));
        }
      }
    }
    /*****************************/
#ifdef DYN_RAM_CNT
    DYN_RAM_POP();
#endif
    /*****************************/

  }
  /*****************************/
#ifdef DYN_RAM_CNT
  DYN_RAM_POP();
#endif
  /*****************************/

  return clas;
}



/*----------------------------------------------------------------------
* G722PLC_syn_filt(m, a, x, y, n n)
* LPC synthesis filter
*
* m (i) : LPC order
* a (i) : LPC coefficients
* x (i) : input buffer
* y (o) : output buffer
* n (i) : number of samples
*---------------------------------------------------------------------- */

static void G722PLC_syn_filt(Word16 m, Word16* a, Word16* x, Word16* y, Word16 n)
{
  Word32 L_temp;
  Word16 j;

#ifdef DYN_RAM_CNT
  {
    UWord32 ssize = 0;
    ssize += (UWord32) ((0) * SIZE_Ptr);
    ssize += (UWord32) ((1) * SIZE_Word16);
    ssize += (UWord32) ((1) * SIZE_Word32);

    DYN_RAM_PUSH(ssize, "dummy");
  }
#endif
  L_temp = L_mult(a[0], *x);  /* Q28= Q12 * Q15 * 2 */
  FOR (j = 1; j <= m; j++)
  {
    L_temp = L_msu(L_temp, a[j], y[-j]);  /* Q28= Q12 * Q15 * 2 */
  }
  *y = round_fx_L_shl(L_temp, 3);
  move16();

  /*****************************/
#ifdef DYN_RAM_CNT
  DYN_RAM_POP();
#endif
  /*****************************/
  return;
}


/*----------------------------------------------------------------------
* G722PLC_ltp_pred_1s(cur_exc, t0, jitter)
* one-step LTP prediction and jitter update
*
* exc     (i)   : excitation buffer (exc[...-1] correspond to past)
* t0      (i)   : pitch lag
* jitter  (i/o) : pitch lag jitter
*---------------------------------------------------------------------- */

static Word16 G722PLC_ltp_pred_1s(Word16* exc, Word16 t0, Word16 *jitter)
{
  Word16 i;
#ifdef DYN_RAM_CNT
  {
    UWord32 ssize = 0;
    ssize += (UWord32) ((0) * SIZE_Ptr);
    ssize += (UWord32) ((1) * SIZE_Word16);
    ssize += (UWord32) ((0) * SIZE_Word32);

    DYN_RAM_PUSH(ssize, "dummy");
  }
#endif

  i = sub(*jitter, t0);

  /* update jitter for next sample */
  *jitter = negate(*jitter);

  /* prediction =  exc[-t0+jitter] */
  /*****************************/
#ifdef DYN_RAM_CNT
  DYN_RAM_POP();
#endif
  /*****************************/
  return exc[i];
}

/*----------------------------------------------------------------------
* G722PLC_ltp_syn(plc_state, cur_exc, cur_syn, n, jitter)
* LTP prediction followed by LPC synthesis filter
*
* plc_state    (i/o) : PLC state variables
* cur_exc  (i)   : pointer to current excitation sample (cur_exc[...-1] correspond to past)
* cur_syn  (i/o) : pointer to current synthesis sample
* n     (i)      : number of samples
* jitter  (i/o)  : pitch lag jitter
*---------------------------------------------------------------------- */

static void G722PLC_ltp_syn(G722PLC_STATE* plc_state, Word16* cur_exc, Word16* cur_syn, Word16 n, Word16 *jitter)
{
  Word16 i;
#ifdef DYN_RAM_CNT
  {
    UWord32 ssize = 0;
    ssize += (UWord32) ((0) * SIZE_Ptr);
    ssize += (UWord32) ((1) * SIZE_Word16);
    ssize += (UWord32) ((0) * SIZE_Word32);

    DYN_RAM_PUSH(ssize, "dummy");
  }
#endif

  FOR (i = 0; i < n; i++)
  {
    /* LTP prediction using exc[...-1] */
    *cur_exc = G722PLC_ltp_pred_1s(cur_exc, plc_state->t0, jitter);
    move16();

    /* LPC synthesis filter (generate one sample) */
    G722PLC_syn_filt(ORD_LPC, plc_state->a, cur_exc, cur_syn, 1);

    cur_exc++;
    cur_syn++;
  } 

  /*****************************/
#ifdef DYN_RAM_CNT
  DYN_RAM_POP();
#endif
  /*****************************/
  return;
}

/*----------------------------------------------------------------------
* G722PLC_syn(plc_state, syn, n)
* extrapolate missing lower-band signal (PLC)
*
* plc_state (i/o) : PLC state variables
* syn   (o)   : synthesis
* n     (i)   : number of samples
*---------------------------------------------------------------------- */

static void G722PLC_syn(G722PLC_STATE * plc_state, Word16 * syn, Word16 n)
{
  Word16 *buffer_syn; /* synthesis buffer */
  Word16 *buffer_exc; /* excitation buffer */
  Word16 *cur_syn;    /* pointer to current sample of synthesis */
  Word16 *cur_exc;    /* pointer to current sample of excition */
  Word16 *exc;        /* pointer to beginning of excitation in current frame */
  Word16 temp;
  Word16 jitter, dim;

  dim = add(n, plc_state->t0p2);
  /* allocate temporary buffers and set pointers */
  buffer_exc = (Word16 *)calloc(dim, sizeof(Word16));
  buffer_syn = (Word16 *)calloc(2*ORD_LPC, sizeof(Word16)); /* minimal allocations of scratch RAM */
#ifdef DYN_RAM_CNT
  {
    UWord32 ssize = 0;
    ssize += (UWord32) ((5) * SIZE_Ptr);
    ssize += (UWord32) ((3 + dim + 2*ORD_LPC) * SIZE_Word16);
    ssize += (UWord32) ((0) * SIZE_Word32);

    DYN_RAM_PUSH(ssize, "dummy");
  }
#endif

  cur_exc = &buffer_exc[plc_state->t0p2];   /* pointer ! */
  cur_syn = &buffer_syn[ORD_LPC]; /* pointer */

  exc = cur_exc;   /* pointer */

  /* copy memory
  - past samples of synthesis (LPC order)            -> buffer_syn[0]
  - last "pitch cycle" of excitation (t0+2) -> buffer_exc[0]
  */

  mov16(ORD_LPC, plc_state->mem_syn, buffer_syn); /*  */
  mov16(plc_state->t0p2, plc_state->mem_exc + sub(MAXPIT2P1, plc_state->t0p2), buffer_exc);   

  /***************************************************
  * set pitch jitter according to clas information *
  ***************************************************/


  jitter = s_and(plc_state->clas, 1);
  plc_state->t0 = s_or(plc_state->t0, jitter);    /* change even delay as jitter is more efficient for odd delays */

  /*****************************************************
  * generate signal by LTP prediction + LPC synthesis *
  *****************************************************/

  temp = sub(n, ORD_LPC);
  /* first samples [0...ord-1] */
  G722PLC_ltp_syn(plc_state, cur_exc, cur_syn, ORD_LPC, &jitter);

  mov16(ORD_LPC, cur_syn, syn);
  /* remaining samples [ord...n-1] */
  G722PLC_ltp_syn(plc_state, &cur_exc[ORD_LPC], &syn[ORD_LPC], temp, &jitter);

  /* update memory:
  - synthesis for next frame (last LPC-order samples)
  - excitation */

  mov16(ORD_LPC, &syn[temp], plc_state->mem_syn);
  G722PLC_update_mem_exc(plc_state, exc, n);

  /* free allocated memory */
  free(buffer_syn);
  free(buffer_exc);
  /*****************************/
#ifdef DYN_RAM_CNT
  DYN_RAM_POP();
#endif
  /*****************************/

  return;
}


/*-------------------------------------------------------------------------*
* Function G722PLC_lpc *
*--------------------------------------------------------------------------*/
static void G722PLC_lpc(G722PLC_STATE * plc_state, Word16 * mem_speech)
{
  Word16 tmp;

  Word16 cor_h[ORD_LPC + 1];   
  Word16 cor_l[ORD_LPC + 1];   
  Word16 rc[ORD_LPC + 1];   
#ifdef DYN_RAM_CNT
  {
    UWord32 ssize = 0;
    ssize += (UWord32) ((0) * SIZE_Ptr);
    ssize += (UWord32) ((1 + 3 + 3*ORD_LPC) * SIZE_Word16);
    ssize += (UWord32) ((0) * SIZE_Word32);

    DYN_RAM_PUSH(ssize, "dummy");
  }
#endif

  G722PLC_autocorr(&mem_speech[MEMSPEECH_LEN - HAMWINDLEN], cor_h, cor_l, ORD_LPC, HAMWINDLEN);   
  Lag_window(cor_h, cor_l, G722PLC_lag_h, G722PLC_lag_l, ORD_LPC);/* Lag windowing*/
  Levinson(cor_h, cor_l, rc, &tmp, ORD_LPC, plc_state->a);

  /*****************************/
#ifdef DYN_RAM_CNT
  DYN_RAM_POP();
#endif
  /*****************************/

  return;
}


/*-------------------------------------------------------------------------*
* Function G722PLC_residu *
*--------------------------------------------------------------------------*/
static void G722PLC_residu(G722PLC_STATE * plc_state)
{
  Word32 L_temp;
  Word16 *ptr_sig, *ptr_res;
  Word16 i, j;
#ifdef DYN_RAM_CNT
  {
    UWord32 ssize = 0;
    ssize += (UWord32) ((2) * SIZE_Ptr);
    ssize += (UWord32) ((2) * SIZE_Word16);
    ssize += (UWord32) ((1) * SIZE_Word32);

    DYN_RAM_PUSH(ssize, "dummy");
  }
#endif

  ptr_res = plc_state->mem_exc;
  ptr_sig =
    &plc_state->mem_speech[MEMSPEECH_LEN - MAXPIT2P1];   

  FOR (i = 0; i < MAXPIT2P1; i++)
  {
    L_temp = L_mult(ptr_sig[i], plc_state->a[0]);
    FOR (j = 1; j <= ORD_LPC; j++)
    {
      L_temp = L_mac(L_temp, plc_state->a[j], ptr_sig[i - j]);   
    }
    L_temp = L_shl(L_temp, 3);/* Q28 -> Q31 */
    ptr_res[i] = round_fx(L_temp); /*Q31 -> Q15 */
    move16();
  }

  /*****************************/
#ifdef DYN_RAM_CNT
  DYN_RAM_POP();
#endif
  /*****************************/
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

static void G722PLC_qmf_updstat (outcode,decoder,lb_signal,hb_signal,state)
short *outcode;
short *lb_signal;
short *hb_signal;
g722_state     *decoder;
void *state;

{
  Word16  rh;
  Word16  i; 
  G722PLC_STATE * plc_state = (G722PLC_STATE *) state;
  Word16 *endLastOut; 
  Word16 *firstFuture;
  Word16 *filtmem, *filtptr;

  filtmem = (Word16 *)calloc(102, sizeof(Word16));

#ifdef DYN_RAM_CNT
  {
    UWord32 ssize = 0;
    ssize += (UWord32) ((5) * SIZE_Ptr);
    ssize += (UWord32) ((2 + 102) * SIZE_Word16);
    ssize += (UWord32) ((0) * SIZE_Word32);

    DYN_RAM_PUSH(ssize, "dummy");
  }
#endif

  mov16(22, &decoder->qmf_rx_delayx[2], &filtmem[L_FRAME_WB]); /*load memory*/
  filtptr = &filtmem[L_FRAME_WB];
  FOR (i = 0; i < L_FRAME_NB; i++)
  {
    /* filter higher-band */
    rh = G722PLC_hp(&plc_state->mem_hpf_in, &plc_state->mem_hpf_out_hi, &plc_state->mem_hpf_out_lo, *hb_signal,
      G722PLC_b_hp156, G722PLC_a_hp156);

    /* calculate output samples of QMF filter */
    qmf_rx_buf (*lb_signal, rh, &filtptr, &outcode);

    lb_signal++;
    hb_signal++;
  }

  mov16(22, filtmem, &decoder->qmf_rx_delayx[2]); /*save memory*/
  free(filtmem);

  /* reset G.722 decoder */ 
  endLastOut = &(plc_state->mem_speech[MEMSPEECH_LEN - 1]);   
  firstFuture = plc_state->crossfade_buf;

  zero16(7, DLT);

  PLT[1] = shr(endLastOut[0],1);
  PLT[2] = shr(endLastOut[-1],1);

  RLT[1] = endLastOut[0];
  RLT[2] = endLastOut[-1];

  SL = firstFuture[0];
  SZL = shr(firstFuture[0],1);
  move16();
  move16();
  move16();
  move16();
  move16();
  move16();

  /* change scale factors (to avoid overshoot) */
  NBH  = shr(NBH, 1);
  DETH = scaleh(NBH); 
  move16();
  move16();

  /* reset G.722 decoder after muting */
  IF(sub(plc_state->count_att_hb, 160) > 0)
  {
    DETL = 32;
    NBL = 0;
    DETH = 8;
    NBH = 0;
    move16();
    move16();
    move16();
    move16();
  }
  AL[1] = mult_r(AL[1],GAMMA_AL2);   move16(); 
  AL[2] = mult_r(AL[2],GAMMA2_AL2);   move16(); 
  BL[1] = mult_r(BL[1],GAMMA_AL2);   move16(); 
  BL[2] = mult_r(BL[2],GAMMA2_AL2);   move16(); 
  BL[3] = mult_r(BL[3],GAMMA3_AL2);   move16(); 
  BL[4] = mult_r(BL[4],GAMMA4_AL2);   move16(); 
  BL[5] = mult_r(BL[5],GAMMA5_AL2);   move16(); 
  BL[6] = mult_r(BL[6],GAMMA6_AL2);   move16(); 

  /*****************************/
#ifdef DYN_RAM_CNT
  DYN_RAM_POP();
#endif
  /*****************************/

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
