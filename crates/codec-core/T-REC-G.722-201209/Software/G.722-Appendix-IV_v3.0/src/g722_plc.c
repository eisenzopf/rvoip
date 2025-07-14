/* ITU G.722 3rd Edition (2012-09) */

/* ITU-T G722 PLC Appendix IV - Reference C code for fixed-point implementation */
/* Version:       1.2                                                           */
/* Revision Date: Jul 3, 2007                                                   */

/*
   ITU-T G.722 PLC Appendix IV   ANSI-C Source Code
   Copyright (c) 2006-2007
   France Telecom
*/

#include <stdlib.h>
#include <stdio.h>
#include <memory.h>
#include <assert.h>

#include "stl.h"
#include "oper_32b.h" 
#include "g722.h"
#include "g722_plc.h"
#if (WMOPS)
#include "count.h"
#endif

/**********************************
 * declaration of PLC subroutines *
 **********************************/

/* lower-band analysis (main subroutine: G722PLC_ana) */
static void 	 G722PLC_ana(G722PLC_STATE * state, g722_state *decoder);
static Word16  G722PLC_hp50(Word16 *x1, Word16* y1_hi, Word16 *y1_lo, Word16 signal);
static Word16  G722PLC_pitch_ol(Word16 * signal, Word16 *maxco, Word16* overfl_shft);
static Word16  G722PLC_classif_modif(Word16 maxco, Word16 nbl, Word16 nbh, Word16* mem_speech, Word16 l_mem_speech,
                                     Word16* mem_exc, Word16 l_exc, Word16* t0);
static void    G722PLC_lag_window(Word16 * R_h, Word16 * R_l, Word16 ord);
static void    G722PLC_levinson(Word16 R_h[], Word16 R_l[], Word16 rc[], Word16 * stable, Word16 ord, Word16 * a);
static void    G722PLC_autocorr(Word16 * x, Word16 * R_h, Word16 * R_l, Word16 ord, Word16 len);
static void    G722PLC_lpc(G722PLC_STATE * state);
static void    G722PLC_residu(G722PLC_STATE * state);

/* lower-band synthesis (main subroutine: G722PLC_syn) */
static void 	 G722PLC_syn(G722PLC_STATE * state, Word16 * syn, Word16 NumSamples);
static Word16  G722PLC_ltp_pred_1s(Word16* exc, Word16 t0, Word16 *jitter);
static void    G722PLC_ltp_syn(G722PLC_STATE* state, Word16* cur_exc, Word16* cur_syn, Word16 n, Word16 *jitter);
static void    G722PLC_syn_filt(Word16 m, Word16* a, Word16* x, Word16* y, Word16 n);
static void 	 G722PLC_attenuate(G722PLC_STATE * state, Word16 * cur_sig, Word16 * tabout, Word16 NumSamples, 
                                 Word16 * ind, Word16 * weight);
static void 	 G722PLC_attenuate_lin(G722PLC_STATE * state, Word16 fact, Word16 * cur_sig, Word16 * tabout, Word16 NumSamples, 
                                     Word16 * ind, Word16 * weight);
static void    G722PLC_calc_weight(Word16 *ind_weight, Word16 fact1, Word16 fact2p, Word16 fact3p, Word16 * weight);
static void    G722PLC_update_mem_exc(G722PLC_STATE * state, Word16 * cur_sig, Word16 NumSamples);

/* higher-band synthesis */
static Word16* G722PLC_syn_hb(G722PLC_STATE * state);


/***********************************
 * definition of main PLC routines *
 ***********************************/


/*----------------------------------------------------------------------
 * G722PLC_init(l_frame)
 * allocate memory and return PLC state variables
 *
 * l_frame (i) : frame length @ 8kHz
 *---------------------------------------------------------------------- */
void * G722PLC_init(Word16 l_frame)
{
	Word16		i;
  G722PLC_STATE * state;

  /* allocate memory for PLC state */
	state = (G722PLC_STATE *)malloc(sizeof(G722PLC_STATE));
	if(state == NULL)
	{
		exit(-1);
	}

	/* constants */
	state->l_frame = l_frame;
	state->t0SavePlus = (Word16) 2; 
	state->l_exc = add(shl(MAXPIT, 1), 1); /* 2 periods +1 for smoothing*/
	state->l_mem_speech = add(state->l_exc, ORD_LPC);

  /* bad frame indicator */
	state->prev_bfi = 0;
#if (WMOPS)
				move16(); 
#endif

  /* LPC, pitch, signal classification parameters */
  state->a = (Word16 *)calloc(ORD_LPC + 1, sizeof(Word16));
	state->mem_syn = (Word16 *)calloc(ORD_LPC, sizeof(Word16));
#if (WMOPS)
  move16();
  move16();
				move16(); 
				move16(); 
				move16(); 
				move16(); 
#endif
	FOR (i = 0; i < ORD_LPC; i++)
  {
		state->mem_syn[i] = (Word16) 0;
		state->a[i] = (Word16) 0;
#if (WMOPS)
    move16();
				move16(); 
#endif
  }
	state->a[ORD_LPC] = (Word16) 0;  
	state->t0 = (Word16) 0;
	state->clas = G722PLC_WEAKLY_VOICED;
#if (WMOPS)
				move16(); 
				move16(); 
				move16(); 
				move16(); 
				move16(); 
				move16(); 
				move16(); 
#endif

  /* signal buffers */
	state->mem_speech = (Word16 *)calloc(state->l_mem_speech, sizeof(Word16));
	state->mem_speech_hb = (Word16 *)calloc(LEN_HB_MEM, sizeof(Word16)); /*MAXPIT is needed, for complexity reason; LEN_HB_MEM: framelength 20ms*/
	state->mem_exc = (Word16 *)calloc(state->l_exc, sizeof(Word16));
#if (WMOPS)
	move16();
				move16(); 
				move16(); 
#endif
	FOR (i = 0; i < state->l_mem_speech; i++)
	{
		state->mem_speech[i] = (Word16) 0;
#if (WMOPS)
	move16();
#endif
	}
	FOR (i = 0; i < LEN_HB_MEM; i++)
  {
		state->mem_speech_hb[i] = (Word16) 0;
#if (WMOPS)
				move16(); 
#endif
  }
	FOR (i = 0; i < state->l_exc; i++)
  {
		state->mem_exc[i] = (Word16) 0;
#if (WMOPS)
				move16(); 
#endif
	}
	
  /* cross-fading */
	state->count_crossfade = CROSSFADELEN;

  /* higher-band hig-pass filtering */
	state->mem_hpf_in = 0;
	state->mem_hpf_out = 0;
	state->count_hpf = 32767;  /*max value*/

  /* adaptive muting */
	state->count_att = (Word16) 0;
	state->count_att_hb = (Word16) 0;
	state->weight_lb = 32767;
	state->weight_hb = 32767;
  state->inc_att = 1;
	state->fact1 = FACT1_V;
	state->fact2p = FACT2P_V;
	state->fact3p = FACT3P_V;
#if (WMOPS)
	move16();
	move16(); 
	move16(); 
	move16(); 
	move16(); 
	move16(); 
	move16(); 
	move16(); 
#endif
	
	return((void *)state);
}

/*----------------------------------------------------------------------
 * G722PLC_conceal(plc_state, xl, xh, outcode, decoder)
 * extrapolation of missing frame
 *
 * plc_state (i/o) : state variables of PLC
 * xl 			 (o)	 : decoded lower-band
 * xh 			 (o)	 : decoder higher-band
 * outcode	 (o)	 : decoded synthesis
 * decoder	 (i/o) : g722 states (QMF, ADPCM)
 *---------------------------------------------------------------------- */
Word32 G722PLC_conceal(void * plc_state, Word16* outcode, g722_state *decoder)
{
  G722PLC_STATE * state = (G722PLC_STATE *) plc_state;
  Word16	Temp, len;
  Word16 * xl, * xh;

  /***********************
   * reset counter			 *
   ***********************/

  state->count_crossfade = 0;  /* reset counter for cross-fading */
#if (WMOPS)
    move16();
#endif

  /***********************
   * generate lower band *
   ***********************/

  /* check if first missing frame (i.e. if previous frame received)
  	 first missing frame -> analyze past buffer + PLC 
  	 otherwise					 -> PLC
	 */
  len = sub(state->l_mem_speech, state->l_frame);
  xl = &state->mem_speech[len];
  IF(state->prev_bfi == 0)
  {
    state->count_att = 0;  /* reset counter for attenuation in lower band */
	  state->count_att_hb = 0; /* reset counter for attenuation in higher band */
	  state->weight_lb = 32767;
	  state->weight_hb = 32767;
#if (WMOPS)
    move16();
    move16();
	  move16();
	  move16();
#endif

    /**********************************
     * analyze buffer of past samples *
     * - LPC analysis
     * - pitch estimation
     * - signal classification
     **********************************/

    G722PLC_ana(state, decoder);

    /******************************
     * synthesize missing samples *
     ******************************/

    /* set increment for attenuation */
	IF(sub(state->clas,G722PLC_VUV_TRANSITION) == 0)
  {
    /* attenuation in 20 ms */
		state->inc_att = 2; 
		state->fact1 = FACT1_UV;
		state->fact2p = FACT2P_UV;
		state->fact3p = FACT3P_UV;
    Temp = FACT3_UV;
  }
	ELSE
	{
    /* attenuation in 40 ms */
		state->inc_att = 1; 
		state->fact1 = FACT1_V;
		state->fact2p = FACT2P_V;
		state->fact3p = FACT3P_V;
    Temp = FACT2_V;
  }
#if (WMOPS)
  move16();
  move16();
  move16();
  move16();
  move16();
#endif

    IF(sub(state->clas, G722PLC_TRANSIENT) == 0)
    {
      /* attenuation in 10 ms */
  		state->inc_att = 4; 
  		state->fact1 = FACT1_V_R;
  		state->fact2p = FACT2P_V_R;
  		state->fact3p = FACT3P_V_R;
      Temp = 0;
#if (WMOPS)
        move16();
      move16();
      move16();
      move16();
      move16();
#endif
    }

    /* synthesize lost frame, high band */
    xh = G722PLC_syn_hb(state);

    /*shift low band*/
    G722PLC_copy(&state->mem_speech[state->l_frame],state->mem_speech,len); /*shift low band*/

    /* synthesize lost frame, low band directly to state->mem_speech*/
    G722PLC_syn(state, xl, state->l_frame);

    /* synthesize cross-fade buffer (part of future frame)*/
    G722PLC_syn(state, state->crossfade_buf, CROSSFADELEN);

    /* attenuate outputs */
		IF(sub(state->l_frame, 80) == 0) /* mode 10 ms, first lost frame : linear weighting*/
    {
      G722PLC_attenuate_lin(state, state->fact1, xl, xl, state->l_frame, &state->count_att, &state->weight_lb);
      if(sub(state->clas, G722PLC_TRANSIENT) == 0)
      {
        state->weight_lb = 0;
#if (WMOPS)
				move16();
#endif
      }
      G722PLC_attenuate_lin(state, Temp, state->crossfade_buf, state->crossfade_buf, CROSSFADELEN, &state->count_att, &state->weight_lb);
      G722PLC_attenuate_lin(state, state->fact1, xh, xh, state->l_frame, &state->count_att_hb, &state->weight_hb);
    }
    ELSE
    {
      G722PLC_attenuate(state, xl, xl, state->l_frame, &state->count_att, &state->weight_lb);
      G722PLC_attenuate(state, state->crossfade_buf, state->crossfade_buf, CROSSFADELEN, &state->count_att, &state->weight_lb);
      G722PLC_attenuate(state, xh, xh, state->l_frame, &state->count_att_hb, &state->weight_hb);
    }
	}
	ELSE
	{
    G722PLC_copy(&state->mem_speech[state->l_frame],state->mem_speech,len); /*shift*/
    /* copy samples from cross-fading buffer (already generated in previous bad frame decoding)  */
    G722PLC_copy(state->crossfade_buf, xl, CROSSFADELEN);

    /* synthesize rest of lost frame */
    Temp = sub(state->l_frame, CROSSFADELEN);
    IF(Temp > 0)
    {
      G722PLC_syn(state, &xl[CROSSFADELEN], Temp);/* remaining synthese for lost frame*/
      G722PLC_attenuate(state, &xl[CROSSFADELEN], &xl[CROSSFADELEN], Temp, &state->count_att, &state->weight_lb);
	}

    /* synthesize cross-fade buffer (part of future frame) and attenuate output */
    G722PLC_syn(state, state->crossfade_buf, CROSSFADELEN);
    G722PLC_attenuate(state, state->crossfade_buf, state->crossfade_buf, CROSSFADELEN, &state->count_att, &state->weight_lb);

    xh = G722PLC_syn_hb(state);
    G722PLC_attenuate(state, xh, xh, state->l_frame, &state->count_att_hb, &state->weight_hb);
  }

	/************************
	 * generate higher band *
	 ************************/

	/* reset DC-remove filter states for good frame decoding */
  IF(sub(state->count_hpf, 32000) >= 0) /* 4 s good frame HP filtering (offset remove) */
  {
  	state->mem_hpf_in = 0;					/* memory of higher band (input of DC-remove filter) */
  	state->mem_hpf_out = 0; 				/* memory of higher band (output of DC-remove filter) */
#if (WMOPS)
		move16(); 
    move16();
#endif
  }
  state->count_hpf = 0; 				/* reset counter used to activate DC-remove filter in good frame mode  */
#if (WMOPS)
  move16();
#endif


	/*****************************************
	 * QMF synthesis filter and state update *
	 *****************************************/

	G722PLC_qmf_updstat(outcode, state->l_frame, decoder, xl, xh, plc_state);

	return (shl(state->l_frame,1));
}


/*----------------------------------------------------------------------
 * G722PLC_rem_dc(mem_in, mem_out, in)
 * DC removal filter
 *
 * mem_in  (i/o) : filter memory
 * mem_out (i/o) : filter memory
 * in	     (i)   : input sample
 *----------------------------------------------------------------------*/

Word16 G722PLC_rem_dc(Word16 *mem_in, Word16 *mem_out, Word16 in)
{
  Word16 out,temp;

  /* 25656 = 0.783 in Q15: 0 dB gain in pass band */

  temp = mult_r(in,29212);
	out = add(temp, sub(mult_r(*mem_out, 25656), *mem_in));
	*mem_in = temp;
	*mem_out = out;
#if (WMOPS)
	move16();
  move16();
#endif
  return out;
}

/*----------------------------------------------------------------------
 * G722PLC_copy(in, out, n)
 * buffer copy
 *
 * in  (i) : input buffer
 * out (o) : output buffer
 * n	 (i) : number of samples
 *----------------------------------------------------------------------*/

void G722PLC_copy(Word16* in, Word16* out, Word16 n)
{
  Word16 i;
  FOR (i = 0; i < n; i++)
  {
    out[i] = *in++;
#if (WMOPS)
	move16();
#endif
  }
  return;
}

/*----------------------------------------------------------------------
 * G722PLC_clear(plc_state)
 * free memory and clear PLC state variables
 *
 * plc_state (i) : PLC state variables
 *---------------------------------------------------------------------- */
void G722PLC_clear(void * plc_state)
{
	G722PLC_STATE * state = (G722PLC_STATE *) plc_state;

	free(state->mem_speech);
	free(state->mem_speech_hb);
	free(state->mem_exc);
	free(state->a);
	free(state->mem_syn);
	free(state);
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
 * signal	     (i)   : input sample
 *----------------------------------------------------------------------*/

Word16 G722PLC_hp50(Word16 *x1, Word16* y1_hi, Word16 *y1_lo, Word16 signal)
{
  Word32    ACC0, ACC1;

    /*  y[i] =      x[i]   -         x[i-1]    */
    /*                     + 123/128*y[i-1]    */
    ACC0 = L_mult0(signal, G722PLC_b_hp[0]);
    ACC0 = L_mac0(ACC0, *x1, G722PLC_b_hp[1]);
    *x1 = signal;
#if (WMOPS)
		move16();
#endif

    ACC0 = L_mac0(ACC0, *y1_hi, G722PLC_a_hp[1]);
    ACC1 = L_mult0(*y1_lo, G722PLC_a_hp[1]);

    ACC0 = L_shl(ACC0, 2);      /* Q29 --> Q31  */
    ACC0 = L_add(ACC0, L_shr(ACC1, 13));

    L_Extract(ACC0, y1_hi, y1_lo);

    return(round(ACC0));
}



/*----------------------------------------------------------------------
 * G722PLC_syn_hb(state, xh, n)
 * reconstruct higher-band by pitch prediction
 *
 * state (i/o)	 : state variables of PLC
 *---------------------------------------------------------------------- */

static Word16* G722PLC_syn_hb(G722PLC_STATE* state)
{
	Word16	 *ptr;
	Word16	 *ptr2;
	Word16	 mem_t0;
  Word16   tmp, tmp2;

	/* save pitch delay */
	mem_t0 = state->t0;
#if (WMOPS)
			move16();
#endif

	/* if signal is not voiced, cut harmonic structure by forcing a 10 ms pitch */
	if(sub(state->clas, G722PLC_VOICED) != 0) 
	{
		state->t0 = 80;
#if (WMOPS)
	move16();
#endif
	}

	/* reconstruct higher band signal by pitch prediction */
  tmp = sub(state->l_frame,state->t0);
  ptr = state->mem_speech_hb + sub(LEN_HB_MEM, state->t0); /*beginning of copy zone*/
  tmp2 = sub(LEN_HB_MEM, state->l_frame); 
  ptr2 = state->mem_speech_hb + tmp2; /*beginning of last frame in mem_speech_hb*/
  IF(tmp <= 0) /* l_frame <= t0; only possible in 10 ms mode*/
  {
  /* temporary save of new frame in state->mem_speech[0 ...state->l_frame-1] of low_band!! that will be shifted after*/
    G722PLC_copy(ptr, state->mem_speech, state->l_frame);
    G722PLC_copy(ptr2, state->mem_speech_hb, state->l_frame); /*shift 1 frame*/
    G722PLC_copy(state->mem_speech, ptr2, state->l_frame);
  }
  ELSE /*t0 < state->l_frame*/
	{
    G722PLC_copy(ptr2, state->mem_speech_hb, tmp2); /*shift memory*/
    G722PLC_copy(ptr, ptr2, state->t0); /*copy last period*/
		G722PLC_copy(ptr2, &ptr2[state->t0], tmp); /*repeate last period*/
	}
	
	/* restore pitch delay */
	state->t0 = mem_t0;
#if (WMOPS)
		move16();
#endif

	return(ptr2);
}


/*-------------------------------------------------------------------------*
 * G722PLC_attenuate(state, in, out, n, count, weight)
 * linear muting with adaptive slope
 *
 * state (i/o) : PLC state variables
 * in    (i)   : input signal
 * out	 (o)   : output signal = attenuated input signal
 * n	   (i)   : number of samples
 * count (i/o) : counter
 * weight (i/o): muting factor
 *--------------------------------------------------------------------------*/

static void G722PLC_attenuate(G722PLC_STATE* state, Word16* in, Word16* out, Word16 n, Word16 *count, Word16 * weight)
{
	Word16		i;

  FOR (i = 0; i < n; i++)
  {
    /* calculate attenuation factor and multiply */
    G722PLC_calc_weight(count, state->fact1, state->fact2p, state->fact3p, weight);
    out[i] = mult_r(*weight, in[i]);
#if (WMOPS)
    move16();
#endif
    *count = add(*count, state->inc_att);
  }
  return;
}

/*-------------------------------------------------------------------------*
 * G722PLC_attenuate_lin(state, fact, in, out, n, count, weight)
 * linear muting with fixed slope
 *
 * state (i/o) : PLC state variables
 * fact  (i/o) : muting parameter
 * in    (i)   : input signal
 * out	 (o)   : output signal = attenuated input signal
 * n	   (i)   : number of samples
 * count (i/o) : counter
 * weight (i/o): muting factor
 *--------------------------------------------------------------------------*/

static void G722PLC_attenuate_lin(G722PLC_STATE* state, Word16 fact, Word16* in, Word16* out, Word16 n, Word16 *count, Word16 * weight)
{
	Word16		i;

  FOR (i = 0; i < 80; i++)
  {
    /* calculate attenuation factor and multiply */
    *weight = sub(*weight, fact);
    out[i] = mult_r(*weight, in[i]);
#if (WMOPS)
    move16();
#endif
    }
  *count = add(*count, i_mult(state->inc_att, 80));
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
				*weight = 0;
#if (WMOPS)
        move16();
#endif
      }
  if(*weight <= 0)
  {
    *ind_weight = END_3RD_PART;
#if (WMOPS)
				move16();
#endif
    }
  return;
}

/*-------------------------------------------------------------------------*
* Function G722PLC_update_mem_exc                                          *
* Update of state->mem_exc and shifts the memory                           *
* if state->t0 > state->l_frame                                            *
*--------------------------------------------------------------------------*/
static void G722PLC_update_mem_exc(G722PLC_STATE * state, Word16 * exc, Word16 n)
{
	Word16	 *ptr;
	Word16		temp;
	Word16		lag;

	/* shift ResMem, if t0 > l_frame */
	lag = add(state->t0, state->t0SavePlus); /*update temp samples*/
	temp = sub(lag, n);

	ptr = state->mem_exc + sub(state->l_exc, lag);
	IF (temp > 0)
  {
    G722PLC_copy(&ptr[n],ptr,temp);
    G722PLC_copy(exc, &ptr[temp], n);
	}
	ELSE
    {
		/* copy last "pitch cycle" of residual */
    G722PLC_copy(&exc[sub(n, lag)], ptr, lag);
  }
  return;
}

/*-------------------------------------------------------------------------*
 * Function G722PLC_ana(state, decoder)									                   *
 * Main analysis routine
 *
 * state   (i/o) : PLC state variables
 * decoder (i)   : G.722 decoder state variables
 *-------------------------------------------------------------------------*/

static void G722PLC_ana(G722PLC_STATE * state, g722_state *decoder)
{  
	Word16 maxco, overfl_shft;

	/* estimate (open-loop) pitch */
	state->t0 = G722PLC_pitch_ol(state->mem_speech + state->l_mem_speech - MAXPIT2,
																 &maxco, &overfl_shft);
#if (WMOPS)
  move16();
#endif

  /* perform LPC analysis and compute residual signal */
  G722PLC_lpc(state);
  G722PLC_residu(state);

  /* update memory for LPC
     during ereased period the state->mem_syn contains the non weighted
     synthetised speech memory. For the	first erased frame, it
     should contain the output speech.
     Saves the last ORD_LPC samples of the output signal in
     state->mem_syn    */
  G722PLC_copy(&state->mem_speech[sub(state->l_mem_speech, ORD_LPC)],
               state->mem_syn, ORD_LPC);

  /* determine signal classification and modify residual in case of transient */
	state->clas = G722PLC_classif_modif(maxco, decoder->nbl, decoder->nbh, state->mem_speech, state->l_mem_speech,
                                        state->mem_exc, state->l_exc, &state->t0);
#if (WMOPS)
  move16();
#endif

	return;
}


/*-------------------------------------------------------*
 * Function G722PLC_lag_window()																 *
 *																											 *
 * G722PLC_lag_window on G722PLC_autocorrelations.											 *
 *																											 *
 * r[i] *= lag_wind[i]																	 *
 *																											 *
 *	r[i] and lag_wind[i] are in special double precision.*
 *	See "oper_32b.c" for the format 										 *
 *																											 *
 *-------------------------------------------------------*/

static void G722PLC_lag_window(Word16 * R_h, Word16 * R_l, Word16 ord)
{
	Word32		x;
	Word16		i;

	FOR (i = 1; i <= ord; i++)
	{
		x = Mpy_32(R_h[i], R_l[i], G722PLC_lag_h[i - 1], G722PLC_lag_l[i - 1]);
		L_Extract(x, &R_h[i], &R_l[i]);
	}
	return;
}


/*-------------------------------------------------------------------------*
* Function G722PLC_levinson 																	*
*--------------------------------------------------------------------------*/

static void G722PLC_levinson(Word16 R_h[],  /* (i) 		: Rh[M+1] Vector of G722PLC_autocorrelations (msb) */
											 Word16 R_l[],	/* (i)		 : Rl[M+1] Vector of G722PLC_autocorrelations (lsb) */
											 Word16 rc[], /* (o) Q15 : rc[M]	 Reflection coefficients. 				 */
											 Word16 * stable, Word16 ord, Word16 * a)
{
	Word32		t0, t1, t2; 				/* temporary variable */

	Word16	 *A_h;								/* LPC coef. in double prec.	*/
	Word16	 *A_l;								/* LPC coef. in double prec.	*/
	Word16	 *An_h; 							/* LPC coef. for next iteration in double prec.  */
	Word16	 *An_l; 							/* LPC coef. for next iteration in double prec.  */

	Word16		i, j;
	Word16		hi, lo;
	Word16		K_h, K_l; 					/* reflexion coefficient; hi and lo */
	Word16		alp_h, alp_l, alp_exp;	/* Prediction gain; hi lo and exponent */

  i = add(ord,1);
	A_h = (Word16 *)calloc(i, sizeof(Word16));
	A_l = (Word16 *)calloc(i, sizeof(Word16));
	An_h = (Word16 *)calloc(i, sizeof(Word16));
	An_l = (Word16 *)calloc(i, sizeof(Word16));

	/* K = A[1] = -R[1] / R[0] */

	*stable = 0;
#if (WMOPS)
		move16(); 
#endif
	t1 = L_Comp(R_h[1], R_l[1]);
	t2 = L_abs(t1); 							/* abs R[1] */
	t0 = Div_32(t2, R_h[0], R_l[0]);	/* R[1]/R[0] */

	if (t1 > 0)
	{
		t0 = L_negate(t0);					/* -R[1]/R[0] */
			}
	L_Extract(t0, &K_h, &K_l);		/* K in DPF */

	rc[0] = K_h;
#if (WMOPS)
			move16();
#endif

	t0 = L_shr(t0, 4);
	L_Extract(t0, &A_h[1], &A_l[1]);	/* A[1] in DPF */

	/*	Alpha = R[0] * (1-K**2) */

	t0 = Mpy_32(K_h, K_l, K_h, K_l);	/* K*K */
	t0 = L_abs(t0); 							/* Some case <0 !! */
	t0 = L_sub((Word32) 0x7fffffffL, t0); /* 1 - K*K */
	L_Extract(t0, &hi, &lo);
	t0 = Mpy_32(R_h[0], R_l[0], hi, lo);	/* Alpha in DPF format */

	/* Normalize Alpha */

	alp_exp = norm_l(t0);
	t0 = L_shl(t0, alp_exp);
	L_Extract(t0, &alp_h, &alp_l);	/* DPF format */

	/*-------------------------------------- */
	/* ITERATIONS  I=2 to lpc_order */
	/*-------------------------------------- */

	FOR (i = 2; i <= ord; i++)
	{
		/* t0 = SUM ( R[j]*A[i-j] ,j=1,i-1 ) +	R[i] */

    t0 = Mpy_32(R_h[1], R_l[1], A_h[i - 1], A_l[i - 1]);
		FOR (j = 2; j < i; j++)
		{
			t0 = L_add(t0, Mpy_32(R_h[j], R_l[j], A_h[i - j], A_l[i - j]));
	}
		t0 = L_shl(t0, 4);

		t1 = L_Comp(R_h[i], R_l[i]);
		t0 = L_add(t0, t1); 				/* add R[i] */

		/* K = -t0 / Alpha */

		t1 = L_abs(t0);
		t2 = Div_32(t1, alp_h, alp_l);	/* abs(t0)/Alpha */

		if (t0 > 0)
    {
			t2 = L_negate(t2);				/* K =-t0/Alpha */
    }
		t2 = L_shl(t2, alp_exp);		/* denormalize; compare to Alpha */
		L_Extract(t2, &K_h, &K_l);	/* K in DPF */
		rc[i - 1] = K_h;
#if (WMOPS)
		move16();
#endif
		/* Test for unstable filter. If unstable keep old A(z) */

		IF (sub(abs_s(K_h), 32750) > 0)
		{
			*stable = 1;
#if (WMOPS)
			move16();
#endif
			free(A_h);
			free(A_l);
			free(An_h);
			free(An_l);
			return;
		}
		/*------------------------------------------ */
		/*	Compute new LPC coeff. -> An[i] 				 */
		/*	An[j]= A[j] + K*A[i-j]		 , j=1 to i-1  */
		/*	An[i]= K																 */
		/*------------------------------------------ */

		FOR (j = 1; j < i; j++)
		{
			t0 = Mpy_32(K_h, K_l, A_h[i - j], A_l[i - j]);
			t0 = L_add(t0, L_Comp(A_h[j], A_l[j]));
			L_Extract(t0, &An_h[j], &An_l[j]);
		}
		t2 = L_shr(t2, 4);
		L_Extract(t2, &An_h[i], &An_l[i]);

		/*	Alpha = Alpha * (1-K**2) */

		t0 = Mpy_32(K_h, K_l, K_h, K_l);	/* K*K */
		t0 = L_abs(t0); 						/* Some case <0 !! */
		t0 = L_sub((Word32) 0x7fffffffL, t0); /* 1 - K*K */
		L_Extract(t0, &hi, &lo);		/* DPF format */
		t0 = Mpy_32(alp_h, alp_l, hi, lo);

		/* Normalize Alpha */

		j = norm_l(t0);
		t0 = L_shl(t0, j);
		L_Extract(t0, &alp_h, &alp_l);	/* DPF format */
		alp_exp = add(alp_exp, j);	/* Add normalization to alp_exp */

		/* A[j] = An[j] */

		FOR (j = 1; j <= i; j++)
		{
			A_h[j] = An_h[j];
			A_l[j] = An_l[j];
#if (WMOPS)
			move16();
			move16();
#endif
		}
	}

	a[0] = 4096;
#if (WMOPS)
	move16();
#endif
  FOR (i = 1; i <= ord; i++)
	{
		t0 = L_Comp(A_h[i], A_l[i]);
		a[i] = round(L_shl(t0, 1));
#if (WMOPS)
		move16();
#endif
	}

	free(A_h);
	free(A_l);
	free(An_h);
	free(An_l);
	return;
}


/*-------------------------------------------------------------------------*
* Function G722PLC_autocorr																*
*--------------------------------------------------------------------------*/

void G722PLC_autocorr(Word16 x[],  /* (i)    : Input signal                      */
                             Word16 r_h[],/* (o)    : Autocorrelations  (msb)           */
                             Word16 r_l[], /* (o)    : Autocorrelations  (lsb)           */
                             Word16 ord,    /* (i)    : LPC order                         */
                             Word16 len /* (i)    : length of analysis                   */
    )
{
  Word32    sum;
  Word16    i, j, norm, tmp;
	Word16 *	y = (Word16 *)calloc(len, sizeof(Word16));

  /* Windowing of signal */

  FOR(i = 0; i < len; i++)
  {
    y[i] = mult_r(x[i], G722PLC_lpc_win_80[HAMWINDLEN-len+i]); /* for length < 80, uses the end of the window */
#if (WMOPS)
    move16();
#endif
  }

  /* Compute r[0] and test for overflow */

  DO
  {
    Overflow = 0;
#if (WMOPS)
    move16();
#endif
    sum = L_mac(1, y[0], y[0]); /*  1 for Avoid case of all zeros */
    FOR(i = 1; i < len; i++)
    {
      sum = L_mac(sum, y[i], y[i]);
    }

    /* If overflow divide y[] by 4 */

    IF(Overflow != 0)
    {
      FOR(i = 0; i < len; i++)
      {
        y[i] = shr(y[i], 2);
#if (WMOPS)
        move16();
#endif
      }
    }
  }
  WHILE(Overflow != 0);

  /* Normalization of r[0] */

  norm = norm_l(sum);
  sum = L_shl(sum, norm);
  L_Extract(sum, &r_h[0], &r_l[0]); /* Put in DPF format (see oper_32b) */

  /* r[1] to r[m] */

  FOR(i = 1; i <= ord; i++)
  {
    sum = L_mult(y[0], y[i]);
    tmp = sub(len, i);
    FOR(j = 1; j < tmp; j++)
    {
      sum = L_mac(sum, y[j], y[j + i]);
    }
    sum = L_shl(sum, norm);
    L_Extract(sum, &r_h[i], &r_l[i]);
  }
  free(y);
  return;
}


/*----------------------------------------------------------------------
 * G722PLC_pitch_ol(signal, length, maxco, overfl_shft)
 * open-loop pitch estimation
 *
 * signal      (i) : pointer to signal buffer (including signal memory)
 * length      (i) : length of signal memory
 * maxco       (o) : maximal correlation
 * overlf_shft (o) : number of shifts
 *
 *---------------------------------------------------------------------- */

static Word16 G722PLC_pitch_ol(Word16 * signal, Word16 *maxco, Word16* overfl_shft)
{  
	Word16	i, j, il, k; 
	Word16	ind, ind2;
	Word16 *w_ds_sig;
	Word32 corx_f, ener1_f, ener2_f;
	Word32 temp_f;
	Word16 valid = 0; /*not valid for the first lobe */
	Word16 start_ind, end_ind, beg_last_per, end_last_per;
	Word16 e1, e2, co, em, norm_e, ne1;
	Word32 ener1n, ener2n;
  Word16 x1, y1_hi, y1_lo;
  Word16 *ptr1, *nooffsigptr;
  Word32 L_temp; 
  Word16 maxco_s8, stable;

	Word16 nooffsig[MAXPIT2+FEC_L_FIR_FILTER_LTP_M1];
	Word16 ds_sig[MAXPIT2_DS];
  Word16 ai[3], cor_h[3], cor_l[3], rc[3];

  /* intialize memory of DC-remove filter */
	FOR (i = 0; i < FEC_L_FIR_FILTER_LTP_M1; i++)
	{
		nooffsig[i] = (Word16) 0;
#if (WMOPS)
		move16();
#endif
	}
	nooffsigptr = nooffsig + FEC_L_FIR_FILTER_LTP_M1;

	/* DC-remove filter */
  x1 = y1_hi = y1_lo = 0;
#if (WMOPS)
  move16();
  move16();
  move16(); 
#endif
	FOR(i = 0; i < MAXPIT2; i++)
	{
    nooffsigptr[i] = G722PLC_hp50(&x1, &y1_hi, &y1_lo, signal[i]);
#if (WMOPS)
		move16(); 
#endif
  }

  /* downsample (filter and decimate) signal */
	ptr1 = ds_sig;
	FOR(i = FACT_M1; i < MAXPIT2; i += FACT)
	{
			temp_f = L_mult0(nooffsigptr[i], G722PLC_fir_lp[0]);
			FOR (k = 1; k < FEC_L_FIR_FILTER_LTP; k++)
			{
				temp_f = L_mac0(temp_f, nooffsigptr[sub(i, k)], G722PLC_fir_lp[k]);
			}
			*ptr1++ = round(temp_f);
#if (WMOPS)
			move16();
#endif
	}

		G722PLC_autocorr(ds_sig, cor_h, cor_l, 2, MAXPIT2_DS);
		G722PLC_lag_window(cor_h, cor_l, 2);	/* Lag windowing		*/
		G722PLC_levinson(cor_h, cor_l, rc, &stable, 2, ai);
		ai[1] = round(L_mult(ai[1],GAMMA));
		ai[2] = round(L_mult(ai[2],GAMMA2));
#if (WMOPS)
		move16();
		move16();
#endif
		/* filter */
    w_ds_sig = (Word16 *)calloc(MAXPIT2_DS, sizeof(Word16));
		w_ds_sig[0] = ds_sig[0];
#if (WMOPS)
		move16();
#endif
		L_temp = L_mult(ai[1], ds_sig[0]);
		w_ds_sig[1] = add(ds_sig[1], round(L_shl(L_temp,3))); 
#if (WMOPS)
		move16();
#endif
		FOR (i = 2; i < MAXPIT2_DS; i++)
		{
      L_temp = L_mult(ai[1], ds_sig[i - 1]);
      L_temp = L_mac(L_temp, ai[2], ds_sig[i - 2]);
			w_ds_sig[i] = add(ds_sig[i], round(L_shl(L_temp,3))); 
#if (WMOPS)
			move16();
#endif
		}
		
	ind = MAXPIT_S2_DS; /*default value*/
	ind2 = 0;

#if (WMOPS)
	move16();
	move16();
#endif

	/*Test overflow on w_ds_sig*/
	*overfl_shft = 0; 
#if (WMOPS)
	move16();
#endif

  /* compute energy of signal in range [len/fac-1,(len-MAX_PIT)/fac-1] */
	ener1_f = 1;
#if (WMOPS)
	move32();
#endif
	FOR(j = MAXPIT2_DSM1; j >= MAXPIT_DSP1; j--)
	{
		ener1_f = L_mac0(ener1_f, w_ds_sig[j], w_ds_sig[j]);
	}

  /* compute exponent */
	ne1 = norm_l(ener1_f);

  /* compute maximal correlation (maxco) and pitch lag (ind) */
	*maxco = 0;
#if (WMOPS)
	move16();
#endif
	ener2_f = L_msu0(ener1_f, w_ds_sig[MAXPIT2_DSM1], w_ds_sig[MAXPIT2_DSM1]); /*update, part 1*/
	FOR(i = 1; i < MAXPIT_DS; i++) /* < to avoid out of range later*/
	{
		ind2 = add(ind2, 1);
		corx_f = 0;
#if (WMOPS)
		move32();
#endif
		
		FOR(j = MAXPIT2_DSM1; j >= MAXPIT_DSP1; j--)
		{
			corx_f = L_mac0(corx_f, w_ds_sig[j], w_ds_sig[j-i]);
		}
		ener2_f = L_mac0(ener2_f, w_ds_sig[MAXPIT_DSP1-i], w_ds_sig[MAXPIT_DSP1-i]); /*update, part 2*/
		norm_e = s_min(ne1, norm_l(ener2_f));
		ener1n = L_shl(ener1_f, norm_e);
		ener2n = L_shl(ener2_f, norm_e);
		corx_f = L_shl(corx_f, norm_e);
		e1 = round(ener1n);
		e2 = round(ener2n);
		ener2_f = L_msu0(ener2_f, w_ds_sig[MAXPIT2_DSM1-i], w_ds_sig[MAXPIT2_DSM1-i]);		 /*update, part 1*/
		co = round(corx_f);
		em = s_max(e1, e2);
        if(sub(co, em) > 0)
        {
          em = co;
          move16();
        }
		if(co > 0)	
    {
      co = div_s(co, em);
    }
		
		if(co < 0) 
		{
			valid = 1;
#if (WMOPS)
			move16();
#endif
		}

    IF(sub(valid,1) == 0)
    {
      
#if (WMOPS)
      test();
#endif
      IF((sub(ind2, ind) == 0) || (sub(ind2, shl(ind,1)) == 0))
      {
        if(sub(*maxco, 27850) > 0) /* 0.85 : high correlation, small chance that double pitch is OK*/
        {
          *maxco = 32767; 
#if (WMOPS)
          move16();
#endif
        }
        
        maxco_s8 = shr(*maxco, 3);
        if(sub(*maxco, 29126) < 0)/*to avoid overflow*/
        {
          *maxco = add(*maxco, maxco_s8); 
        }
      }
      
#if (WMOPS)
      test();
#endif
      IF((sub(co, *maxco) > 0) && (sub(i, MINPIT_DS) >= 0))
      {
        *maxco = co;
        ind = i;
        ind2 = 1;
        
#if (WMOPS)
        move16();
        move16();
        move16();
#endif
      }
    }
	}
	free(w_ds_sig);

  /* convert pitch to non decimated domain */	
	il = shl(ind, FACTLOG2);
	ind = il;
#if (WMOPS)
	move16();
#endif

  /* shift DC-removed signal to avoid overflow in correlation */
	if(L_sub(ener1_f, 0x01000000) > 0) /* maxcor will be computed on 4* points in non weighted domain --> overflow risq*/
	{
		*overfl_shft = add(*overfl_shft,1);
	}

	IF(*overfl_shft > 0)
	{
		FOR(i = 1; i < MAXPIT2; i++)
		{
			nooffsigptr[i] = shr(nooffsigptr[i], *overfl_shft);
#if (WMOPS)
			move16();
#endif
		}
	}
	
  /* refine pitch in non-decimated (8 kHz) domain by step of 1
     -> maximize correlation around estimated pitch lag (ind) */
	start_ind = sub(il, 2);
  start_ind = s_max(start_ind, MINPIT);
	end_ind = add(il, 2);
	beg_last_per = sub(MAXPIT2, il);
	end_last_per = sub(MAXPIT2, 1);
  j = sub(end_last_per, start_ind);
  ener1_f = L_mac0(1, nooffsigptr[end_last_per], nooffsigptr[end_last_per]); /*to avoid division by 0*/
  ener2_f = L_mac0(1, nooffsigptr[j], nooffsigptr[j]); /*to avoid division by 0*/
  FOR(j = sub(end_last_per, 1); j >= beg_last_per; j--)
  {
    ener1_f	= L_mac0(ener1_f, nooffsigptr[j], nooffsigptr[j]);
    ener2_f	= L_mac0(ener2_f, nooffsigptr[j-start_ind], nooffsigptr[j-start_ind]);
  }
  j = sub(beg_last_per, start_ind);
	ener2_f = L_msu0(ener2_f, nooffsigptr[j], nooffsigptr[j]); /*to compansate first update part 2*/
  /* compute exponent */
	ne1 = norm_l(ener1_f);
  /* compute maximal correlation (maxco) and pitch lag (ind) */
	*maxco = 0;
#if (WMOPS)
	move16();
#endif

	FOR(i = start_ind; i <= end_ind; i++)
	{
		corx_f = 0;
#if (WMOPS)
		move32();
#endif
		
		ener2_f = L_mac0(ener2_f, nooffsigptr[beg_last_per-i], nooffsigptr[beg_last_per-i]); /*update, part 2*/
		FOR(j = end_last_per; j >= beg_last_per; j--)
		{
			corx_f	= L_mac0(corx_f, nooffsigptr[j], nooffsigptr[j-i]);
		}
    norm_e = s_min(ne1, norm_l(ener2_f));
    ener1n = L_shl(ener1_f, norm_e);
    ener2n = L_shl(ener2_f, norm_e);
    corx_f = L_shl(corx_f, norm_e);
    e1 = round(ener1n);
    e2 = round(ener2n);
    co = round(corx_f);
    em = s_max(e1, e2);
    if(sub(co, em) > 0)
    {
      em = co;
      move16();
    }
    if(co > 0)	
    {
      co = div_s(co, em);
    }
		
		if(sub(co, *maxco) > 0)
		{
			ind = i;
#if (WMOPS)
			move16();
#endif
		}
    *maxco = s_max(co, *maxco);
    ener2_f = L_msu0(ener2_f, nooffsigptr[end_last_per-i], nooffsigptr[end_last_per-i]);		 /*update, part 1*/
        
	}
	
	IF (sub(*maxco, 8192) < 0)
	{
		if(sub(ind, 32) < 0)
		{
			ind = shl(ind,1); /*2 times pitch for very small pitch, at least 2 times MINPIT */
		}
		
	}

	return ind;
}


/*----------------------------------------------------------------------
 * G722PLC_classif_modif(maxco, overfl_shft, decoder)
 * signal classification and conditional residual modification
 *
 * maxco       (i) : maximal correlation
 * nbl         (i) : lower-band G722 scale factor
 * nbh         (i) : higher-band G722 scale factor
 * mem_speech  (i) : pointer to speech buffer
 * l_mem_speech(i) : length of speech buffer
 * mem_exc     (i) : pointer to excitation buffer
 * l_exc       (i) : length of excitation buffer
 * t0          (i) : open-loop pitch
 *---------------------------------------------------------------------- */

static Word16 G722PLC_classif_modif(Word16 maxco, Word16 nbl, Word16 nbh, Word16* mem_speech, Word16 l_mem_speech,
                                    Word16* mem_exc, Word16 l_exc, Word16* t0)

{
  Word16 clas, Temp, tmp1, tmp2, tmp3, tmp4, i, maxres, absres, zcr, clas2;
  Word16 *pt1, *pt2;

  /************************************************************************
	 * select preliminary class => clas = UNVOICED, WEAKLY_VOICED or VOICED *
   * by default clas=WEAKLY_VOICED                         								*
   * classification criterio:                                             *
   * -normalized correlation from open-loop pitch                         *
   * -ratio of lower/higher band energy (G722 scale factors)              *
   * -zero crossing rate                                                  *
	 ************************************************************************/

  /* compute zero-crossing rate in last 10 ms */
  pt1 = &mem_speech[sub(l_mem_speech, 80)];
  pt2 = pt1-1;
  zcr = 0;
#if (WMOPS)
  move16();
#endif
  FOR(i = 0; i< 80; i++)
  {
    Temp = 0;
#if(WMOPS)
    move16();
#endif
    if(*pt1 <= 0)
    {
      Temp = 1;
#if(WMOPS)
      move16();
#endif
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
#if (WMOPS)
  move16();
#endif

  /* detect voiced clas (corr > 3/4 ener1 && corr > 3/4 ener2) */
  if(sub(maxco, 22936) > 0) /* 22936 in Q15 = 0.7 */
  {
    clas = G722PLC_VOICED;
#if (WMOPS)
    move16();
#endif
  }

  /* change class to unvoiced if higher band has lots of energy
     (risk of "dzing" if clas is "very voiced") */
  IF(sub(nbh, nbl) > 0)
  {
    clas2 = clas;
    clas = G722PLC_VUV_TRANSITION;
#if (WMOPS)
    move16();
    move16();
#endif

    if(sub(clas2, G722PLC_VOICED) == 0)
    {
      clas = G722PLC_WEAKLY_VOICED;
#if (WMOPS)
    move16();
#endif
    }
  }

  /* change class to unvoiced if zcr is high */
  IF (sub(zcr,20)>=0)
  {
    clas = G722PLC_UNVOICED;
#if (WMOPS)
    move16();
#endif
    /* change pitch if unvoiced class (to avoid short pitch lags) */
    if(sub(*t0, 32) < 0)
    {
      *t0 = shl(*t0,1); /*2 times pitch for very small pitch, at least 2 times MINPIT */
    }

  }


	/**************************************************************************
	 * detect transient => clas = TRANSIENT	                                  *
   * + modify residual to limit amplitude for LTP                           *
   * (this is performed only if current class is not VOICED to avoid        *
   *  perturbation of the residual for LTP)                                 *
	 **************************************************************************/

  /* detect transient and limit amplitude of residual */
  Temp = 0;
  IF (sub(clas,4) > 0)/*G722PLC_WEAKLY_VOICED(5) or G722PLC_VUV_TRANSITION(7)*/
  {
    tmp1 = sub(l_exc, *t0); /* tmp1 = start index of last "pitch cycle" */
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
#if (WMOPS)
          move16();
#endif
        }
      }
    }
  }
  if (Temp>0)
  {
     clas = G722PLC_TRANSIENT;
#if (WMOPS)
     move16();
#endif
  }

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

			L_temp = L_mult(a[0], *x);  /* Q28= Q12 * Q15 * 2 */
			FOR (j = 1; j <= m; j++)
			{
				L_temp = L_msu(L_temp, a[j], y[-j]);  /* Q28= Q12 * Q15 * 2 */
			}
			*y = round(L_shl(L_temp, 3));
#if (WMOPS)
			move16();
#endif
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

  i = sub(*jitter, t0);

  /* update jitter for next sample */
  *jitter = negate(*jitter);

  /* prediction =  exc[-t0+jitter] */
  return exc[i];
}


/*----------------------------------------------------------------------
 * G722PLC_ltp_syn(state, cur_exc, cur_syn, n, jitter)
 * LTP prediction followed by LPC synthesis filter
 *
 * state    (i/o) : PLC state variables
 * cur_exc  (i)   : pointer to current excitation sample (cur_exc[...-1] correspond to past)
 * cur_syn  (i/o) : pointer to current synthesis sample
 * n     (i)      : number of samples
 * jitter  (i/o)  : pitch lag jitter
 *---------------------------------------------------------------------- */

static void G722PLC_ltp_syn(G722PLC_STATE* state, Word16* cur_exc, Word16* cur_syn, Word16 n, Word16 *jitter)
{
  Word16 i;

		FOR (i = 0; i < n; i++)
		{
			/* LTP prediction using exc[...-1] */
      *cur_exc = G722PLC_ltp_pred_1s(cur_exc, state->t0, jitter);
#if (WMOPS)
			move16();
#endif
			
			/* LPC synthesis filter (generate one sample) */
      G722PLC_syn_filt(ORD_LPC, state->a, cur_exc, cur_syn, 1);

			cur_exc++;
			cur_syn++;
		} 
    return;
}


/*----------------------------------------------------------------------
 * G722PLC_syn(state, syn, n)
 * extrapolate missing lower-band signal (PLC)
 *
 * state (i/o) : PLC state variables
 * syn   (o)   : synthesis
 * n     (i)   : number of samples
 *---------------------------------------------------------------------- */

static void G722PLC_syn(G722PLC_STATE * state, Word16 * syn, Word16 n)
{
	Word16	 *buffer_syn; /* synthesis buffer */
	Word16	 *buffer_exc; /* excitation buffer */
	Word16	 *cur_syn;    /* pointer to current sample of synthesis */
	Word16	 *cur_exc;    /* pointer to current sample of excition */
	Word16	 *exc;        /* pointer to beginning of excitation in current frame */
	Word16		temp;
	Word16		jitter, dim;

  dim = add(n, add(state->t0, state->t0SavePlus));
  /* allocate temporary buffers and set pointers */
  buffer_exc = (Word16 *)calloc(dim, sizeof(Word16));
	buffer_syn = (Word16 *)calloc(2*ORD_LPC, sizeof(Word16)); //minimal allocations of scratch RAM

  cur_exc = &buffer_exc[add(state->t0, state->t0SavePlus)];
	cur_syn = &buffer_syn[ORD_LPC];

	exc = cur_exc;

  /* copy memory
     - past samples of synthesis (LPC order)            -> buffer_syn[0]
     - last "pitch cycle" of excitation (t0+t0SavePlus) -> buffer_exc[0]
  */
  G722PLC_copy(state->mem_syn, buffer_syn, ORD_LPC); /*  */

	temp = add(state->t0, state->t0SavePlus);
  G722PLC_copy(state->mem_exc + sub(state->l_exc, temp), buffer_exc, temp);

  /***************************************************
   * set pitch jitter according to clas information *
   ***************************************************/


  jitter = s_and(state->clas, 1);
  state->t0 = s_or(state->t0, jitter);    /* change even delay as jitter is more efficient for odd delays */

  /*****************************************************
   * generate signal by LTP prediction + LPC synthesis *
   *****************************************************/

  temp = sub(n, ORD_LPC);
  /* first samples [0...ord-1] */
  G722PLC_ltp_syn(state, cur_exc, cur_syn, ORD_LPC, &jitter);
  G722PLC_copy(cur_syn, syn, ORD_LPC);
  
  /* remaining samples [ord...n-1] */
  G722PLC_ltp_syn(state, &cur_exc[ORD_LPC], &syn[ORD_LPC], temp, &jitter);

  /* update memory:
     - synthesis for next frame (last LPC-order samples)
     - excitation */
  G722PLC_copy(&syn[temp], state->mem_syn, ORD_LPC);
	G722PLC_update_mem_exc(state, exc, n);

  /* free allocated memory */
	free(buffer_syn);	
	free(buffer_exc);
	return;
}


/*-------------------------------------------------------------------------*
* Function G722PLC_lpc																	 *
*--------------------------------------------------------------------------*/
static void G722PLC_lpc(G722PLC_STATE * state)
{
	Word16		tmp;

	Word16		cor_h[ORD_LPC + 1];
	Word16		cor_l[ORD_LPC + 1];
	Word16		rc[ORD_LPC + 1];

	G722PLC_autocorr(&state->mem_speech[state->l_mem_speech - HAMWINDLEN], cor_h, cor_l, ORD_LPC, HAMWINDLEN);
	G722PLC_lag_window(cor_h, cor_l, ORD_LPC);	/* Lag windowing		*/
	G722PLC_levinson(cor_h, cor_l, rc, &tmp, ORD_LPC, state->a);

#ifdef USE_FEC_PRINTF
	if (stable != 0)
	{
		fprintf(stderr, "WARNING: LPC filter unstable !!!\n");
		/*exit(0); */
	}
#endif

	return;
}


/*-------------------------------------------------------------------------*
* Function G722PLC_residu 																*
*--------------------------------------------------------------------------*/
static void G722PLC_residu(G722PLC_STATE * state)
{
	Word32		L_temp;
	Word16	 *ptr_sig, *ptr_res;
	Word16		i, j;

	ptr_res = state->mem_exc + sub(state->l_exc, MAXPIT2P1);
	ptr_sig =
			&state->mem_speech[state->l_mem_speech - MAXPIT2P1];

	FOR (i = 0; i < MAXPIT2P1; i++)
	{
		L_temp = L_mult(ptr_sig[i], state->a[0]);
		FOR (j = 1; j <= ORD_LPC; j++)
		{
			L_temp = L_mac(L_temp, state->a[j], ptr_sig[i - j]);
		}
		L_temp = L_shl(L_temp, 3);	/* Q28 -> Q31 */
		ptr_res[i] = round(L_temp); /*Q31 -> Q15 */
#if (WMOPS)
		move16();
#endif
	}

	return;
}


