/* ITU G.722 3rd Edition (2012-09) */

/* ITU-T G722 PLC Appendix IV - Reference C code for fixed-point implementation */
/* Version:       1.2                                                           */
/* Revision Date: Jul 3, 2007                                                   */

/*
   ITU-T G.722 PLC Appendix IV   ANSI-C Source Code
   Copyright (c) 2006-2007
   France Telecom
*/

#ifndef __G722_MAIN_ERRORC_H__
#define __G722_MAIN_ERRORC_H__

#include <stdio.h>
#include "g722.h"

/*********************
 * Constants for PLC *
 *********************/

/* signal classes */
#define G722PLC_TRANSIENT		    (Word16)3
#define G722PLC_UNVOICED		    (Word16)1
#define G722PLC_VUV_TRANSITION  (Word16)7
#define G722PLC_WEAKLY_VOICED   (Word16)5
#define G722PLC_VOICED          (Word16)0

/* 4:1 decimation constants */
#define FACT                    4                /* decimation factor for pitch analysis */
#define FACTLOG2				        2                /* log2(FACT) */
#define FACT_M1					        (FACT-1)
#define FACT_S2					        (FACT/2)
#define FEC_L_FIR_FILTER_LTP	  9                /* length of decimation filter */
#define FEC_L_FIR_FILTER_LTP_M1	(FEC_L_FIR_FILTER_LTP-1) /* length of decimation filter - 1 */

/* open-loop pitch parameters */
#define MAXPIT					        144              /* maximal pitch lag (20ms @8kHz) => 50 Hz */
#define MAXPIT2					        (2*MAXPIT)
#define MAXPIT2P1				        (MAXPIT2+1)
#define MAXPIT_S2				        (MAXPIT/2)
#define MAXPIT_P2				        (MAXPIT+2)
#define MAXPIT_DS				        (MAXPIT/FACT)
#define MAXPIT_DSP1			        (MAXPIT_DS+1)
#define MAXPIT_DSM1  		        (MAXPIT_DS-1)
#define MAXPIT2_DS				      (MAXPIT2/FACT)
#define MAXPIT2_DSM1				    (MAXPIT2_DS-1)
#define MAXPIT_S2_DS			      (MAXPIT_S2/FACT)
#define MINPIT					        16               /* minimal pitch lag (2ms @8kHz) => 500 Hz */
#define MINPIT_DS				        (MINPIT/FACT)
#define GAMMA                   30802            /* 0.94 in Q15 */
#define GAMMA2                  28954            /* 0.94^2 */

/* LPC windowing */
#define ORD_LPC					        8                /* LPC order */
#define HAMWINDLEN					    80                /* length of the assymetrical hamming window */

/* cross-fading parameters */
#define CROSSFADELEN			      80               /* length of crossfade (10 ms @8kHz) */
#define CROSSFADELEN16			    160              /* length of crossfade (10 ms @16kHz) */

/* adaptive muting parameters */
#define END_1ST_PART  			    80               /* attenuation range: 10ms @ 8kHz */
#define END_2ND_PART  			    160              /* attenuation range: 20ms @ 8kHz */ 
#define END_3RD_PART  			    320              /* attenuation range: 40ms @ 8kHz */ 
#define FACT1_V					        10
#define FACT2_V					        20
#define FACT3_V					        190
#define FACT2P_V					      (FACT2_V-FACT1_V)
#define FACT3P_V				  	    (FACT3_V-FACT2_V)
#define FACT1_UV				  	    10
#define FACT2_UV				  	    10
#define FACT3_UV				  	    399
#define FACT2P_UV				  	    (FACT2_UV-FACT1_UV)
#define FACT3P_UV				  	    (FACT3_UV-FACT2_UV)
#define FACT1_V_R				        409
#define FACT2_V_R				        409
#define FACT3_V_R				        409
#define FACT2P_V_R					    (FACT2_V_R-FACT1_V_R)
#define FACT3P_V_R					    (FACT3_V_R-FACT2_V_R)

/* size of higher-band signal buffer */
#define LEN_HB_MEM              160

/**************
 * PLC states *
 **************/

typedef struct _G722PLC_STATE
{
  Word16    prev_bfi; /* bad frame indicator of previous frame */
  Word16    l_frame;  /* frame length @ 8kHz */

  /* constants (lengths, LPC order, pitch range) */
  Word16    l_mem_speech;   /* length of speech buffer (constant) */
  Word16    l_exc;          /* length of LPC residual buffer (constant) */
  Word16    t0SavePlus;     /* constant*/

  /* signal buffers */
  Word16   *mem_speech;     /* lower-band speech buffer */
  Word16   *mem_exc;        /* past LPC residual */
  Word16   *mem_speech_hb;  /* higher-band speech buffer */

  /* analysis results (signal class, LPC coefficients, pitch delay) */
  Word16    clas;  /* unvoiced, weakly voiced, voiced */
  Word16    t0;    /* pitch delay */

  /* variables for crossfade */
  Word16    count_crossfade; /* counter for cross-fading (number of samples) */
  Word16    crossfade_buf[CROSSFADELEN];

  /* variables for DC remove filter in higher band */
  Word16	count_hpf;
  Word16	mem_hpf_in;
  Word16	mem_hpf_out;

  /* variables for synthesis attenuation */
  Word16   count_att;    /* counter for lower-band attenuation (number of samples) */
  Word16   count_att_hb; /* counter for higher-band attenuation (number of samples) */
  Word16   inc_att;      /* increment for counter update */
  Word16   fact1;
  Word16   fact2p;
  Word16   fact3p;
  Word16   weight_lb;
  Word16   weight_hb;

  /* coefficient of ARMA predictive filter A(z)/B(z) of G.722 */
  Word16   *a;     /* LPC coefficients */
  Word16   *mem_syn;        /* past synthesis */

} G722PLC_STATE;

/**************
 * PLC tables *
 **************/

const Word16 G722PLC_lag_h[ORD_LPC];
const Word16 G722PLC_lag_l[ORD_LPC];
const Word16 G722PLC_lpc_win_80[80];
const Word16 G722PLC_fir_lp[FEC_L_FIR_FILTER_LTP];
const Word16 G722PLC_b_hp[2];
const Word16 G722PLC_a_hp[2];

/****************
 * PLC routines *
 ****************/

void*	 G722PLC_init(Word16 l_frame);
Word32 G722PLC_conceal(void * plc_state, Word16* outcode, g722_state *decoder);
Word16 G722PLC_rem_dc(Word16 *mem_in, Word16 *mem_out, Word16 in);
void   G722PLC_copy(Word16* in, Word16* out, Word16 n);
void	 G722PLC_clear(void * state);

#endif
