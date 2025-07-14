/* ITU G.722 3rd Edition (2012-09) */

/*--------------------------------------------------------------------------
 ITU-T Annex B (ex G.722-SWB) Source Code
 Software Release 1.00 (2010-09)
 (C) 2010 France Telecom, Huawei Technologies, NTT, VoiceAge Corp.
--------------------------------------------------------------------------*/


#ifndef __G722_MAIN_ERRORC_H__
#define __G722_MAIN_ERRORC_H__

#include "funcg722.h"

#define HP_FILTER_MODIF_FT

/*********************
 * Constants for PLC *
 *********************/

/* signal classes */
#define G722PLC_TRANSIENT       (Word16)3
#define G722PLC_UNVOICED        (Word16)1
#define G722PLC_VUV_TRANSITION  (Word16)7
#define G722PLC_WEAKLY_VOICED   (Word16)5
#define G722PLC_VOICED          (Word16)0

/* 4:1 decimation constants */
#define FACT                      4                /* decimation factor for pitch analysis */
#define FACTLOG2                  2                /* log2(FACT) */
#define FACT_M1                   (FACT-1)	
#define FACT_S2                   (FACT/2)	
#define FEC_L_FIR_FILTER_LTP      9                /* length of decimation filter */	
#define FEC_L_FIR_FILTER_LTP_M1   (FEC_L_FIR_FILTER_LTP-1) /* length of decimation filter - 1 */	
#define NOOFFSIG_LEN              (MAXPIT2+FEC_L_FIR_FILTER_LTP_M1)	
#define MEMSPEECH_LEN             (MAXPIT2+ORD_LPC+1)	

/* open-loop pitch parameters */
#define MAXPIT                    144              /* maximal pitch lag (20ms @8kHz) => 50 Hz */
#define MAXPIT2                   (2*MAXPIT)	

#define END_LAST_PER   (MAXPIT2-1)
#define END_LAST_PER_1 (END_LAST_PER-1)

#define MAXPIT2P1                 (MAXPIT2+1)	
#define MAXPIT_S2                 (MAXPIT/2)	
#define MAXPIT_P2                 (MAXPIT+2)	
#define MAXPIT_DS                 (MAXPIT/FACT)	
#define MAXPIT_DSP1               (MAXPIT_DS+1)	
#define MAXPIT_DSM1               (MAXPIT_DS-1)	
#define MAXPIT2_DS                (MAXPIT2/FACT)	
#define MAXPIT2_DSM1              (MAXPIT2_DS-1)	
#define MAXPIT_S2_DS              (MAXPIT_S2/FACT)	
#define MINPIT                            16               /* minimal pitch lag (2ms @8kHz) => 500 Hz */
#define MINPIT_DS                 (MINPIT/FACT)	
#define GAMMA                     30802            /* 0.94 in Q15 */
#define GAMMA2                    28954            /* 0.94^2 */
#define GAMMA_AL2                 31785            /* 0.97 in Q15 */
#define GAMMA2_AL2                30831            /* 0.99^2 */
#define GAMMA3_AL2                29906            /* 0.99^3 */
#define GAMMA4_AL2                29009            /* 0.99^4 */
#define GAMMA5_AL2                28139            /* 0.99^5 */
#define GAMMA6_AL2                27295            /* 0.99^6 */
#define GAMMA_AZ1                 32440            /* 0.99 in Q15 */
#define GAMMA_AZ2                 32116            /* 0.99^2 in Q15 */
#define GAMMA_AZ3                 31795            /* 0.99^3 in Q15 */
#define GAMMA_AZ4                 31477            /* 0.99^4 in Q15 */
#define GAMMA_AZ5                 31162            /* 0.99^5 in Q15 */
#define GAMMA_AZ6                 30850            /* 0.99^6 in Q15 */
#define GAMMA_AZ7                 30542            /* 0.99^7 in Q15 */
#define GAMMA_AZ8                 30236            /* 0.99^8 in Q15 */

/* LPC windowing */
#define ORD_LPC                   8                /* LPC order */
#define ORD_LPCP1                 9                /* LPC order +1*/
#define HAMWINDLEN                80                /* length of the assymetrical hamming window */

/* cross-fading parameters */
#define CROSSFADELEN              80               /* length of crossfade (10 ms @8kHz) */
#define CROSSFADELEN16            160              /* length of crossfade (10 ms @16kHz) */

/* adaptive muting parameters */
#define END_1ST_PART              80               /* attenuation range: 10ms @ 8kHz */
#define END_2ND_PART              160              /* attenuation range: 20ms @ 8kHz */ 
#define END_3RD_PART              480              /* attenuation range: 60ms @ 8kHz */ 
#define FACT1_V                   10
#define FACT2_V                   20
#define FACT3_V                   95  /*30367/320*/
#define FACT2P_V                  (FACT2_V-FACT1_V)	
#define FACT3P_V                  (FACT3_V-FACT2_V)	
#define FACT1_UV                  10
#define FACT2_UV                  10
#define FACT3_UV                  200 /*31967/160*/
#define FACT2P_UV                 (FACT2_UV-FACT1_UV)	
#define FACT3P_UV                 (FACT3_UV-FACT2_UV)	
#define FACT1_V_R                 409 /*correction 25/09/07 for step by 6, was 273 for step by 4, because problem attenuate_lin with 15 ms*/
#define FACT2_V_R                 409 /*32768/80*/
#define FACT3_V_R                 409
#define FACT2P_V_R                (FACT2_V_R-FACT1_V_R)	
#define FACT3P_V_R                (FACT3_V_R-FACT2_V_R)	
#define LIMIT_FOR_RESET           160 /*with 240 there are some accidents */

/* size of higher-band signal buffer */
#define LEN_HB_MEM                160
#define LEN_HB_MEM_MLF            (LEN_HB_MEM - L_FRAME_NB)

/**************
 * PLC states *
 **************/

typedef struct _G722PLC_STATE
{
  Word16    prev_bfi; /* bad frame indicator of previous frame */
  Word16    l_frame;  /* frame length @ 8kHz */


  /* signal buffers */
  Word16   *mem_speech;     /* lower-band speech buffer */
  Word16   *mem_exc;        /* past LPC residual */
  Word16   *mem_speech_hb;  /* higher-band speech buffer */

  /* analysis results (signal class, LPC coefficients, pitch delay) */
  Word16    clas;  /* unvoiced, weakly voiced, voiced */
  Word16    t0;    /* pitch delay */
  Word16    t0p2;     /* constant*/

  /* variables for crossfade */
  Word16    count_crossfade; /* counter for cross-fading (number of samples) */
  Word16    crossfade_buf[CROSSFADELEN];

  /* variables for DC remove filter in higher band */
  Word16    mem_hpf_in;
  Word16    mem_hpf_out_hi;
  Word16    mem_hpf_out_lo;

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

extern const Word16 G722PLC_lag_h[ORD_LPC];
extern const Word16 G722PLC_lag_l[ORD_LPC];
extern const Word16 G722PLC_lpc_win_80[80];
extern const Word16 G722PLC_fir_lp[FEC_L_FIR_FILTER_LTP];
extern const Word16 G722PLC_b_hp[2];
extern const Word16 G722PLC_a_hp[2];
extern const Word16 G722PLC_b_hp156[2];
extern const Word16 G722PLC_a_hp156[2];
extern const Word16 G722PLC_gamma_az[9];

/****************
 * PLC routines *
 ****************/
void*  G722PLC_init(void);
void G722PLC_conceal(void * plc_state, Word16* outcode, g722_state *decoder);

Word16 G722PLC_hp(Word16 *x1, Word16* y1_hi, Word16 *y1_lo, Word16 signal,
					const Word16 *G722PLC_b_hp, const Word16 *G722PLC_a_hp);

void   G722PLC_clear(void * state);

#endif
