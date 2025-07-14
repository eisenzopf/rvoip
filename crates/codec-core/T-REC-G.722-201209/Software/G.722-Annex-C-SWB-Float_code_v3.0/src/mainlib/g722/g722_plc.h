/* ITU G.722 3rd Edition (2012-09) */

/*--------------------------------------------------------------------------
 ITU-T G.722 Annex C (ex G.722-SWB-Float) Source Code
 Software Release 1.01 (2012-07)
 (C) 2012 France Telecom, Huawei Technologies, VoiceAge Corp., NTT.
--------------------------------------------------------------------------*/


#ifndef __G722_MAIN_ERRORC_H__
#define __G722_MAIN_ERRORC_H__

#include "funcg722.h"

#define HP_FILTER_MODIF_FT

/*********************
 * Constants for PLC *
 *********************/

/* signal classes */
#define G722PLC_TRANSIENT       (Short)3
#define G722PLC_UNVOICED        (Short)1
#define G722PLC_VUV_TRANSITION  (Short)7
#define G722PLC_WEAKLY_VOICED   (Short)5
#define G722PLC_VOICED          (Short)0

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
#define F_GAMMA                   ((Float)0.94)            /* 0.94 in Q15 */
#define F_GAMMA2                  ((Float)0.8836)            /* 0.94^2 */
#define F_GAMMA_AL2                 ((Float)0.97)            /* 0.97 in Q15 */
#define F_GAMMA2_AL2                ((Float)0.9409)            /* 0.97^2 */
#define F_GAMMA3_AL2                ((Float)0.9127)            /* 0.97^3 */
#define F_GAMMA4_AL2                ((Float)0.8853)            /* 0.97^4 */
#define F_GAMMA5_AL2                ((Float)0.8587)            /* 0.97^5 */
#define F_GAMMA6_AL2                ((Float)0.8330)            /* 0.97^6 */
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

#define F_FACT1_V                   (Float)3.0517578e-4
#define F_FACT2_V                   (Float)6.1035156e-4
#define F_FACT3_V                   (Float)2.8991699e-3  /*30367/320*/
#define F_FACT2P_V                  (F_FACT2_V-F_FACT1_V)	
#define F_FACT3P_V                  (F_FACT3_V-F_FACT2_V)	
#define F_FACT1_UV                  (Float)3.0517578e-4
#define F_FACT2_UV                  (Float)3.0517578e-4
#define F_FACT3_UV                  (Float)6.1035156e-3 /*31967/160*/
#define F_FACT2P_UV                 (F_FACT2_UV-F_FACT1_UV)	
#define F_FACT3P_UV                 (F_FACT3_UV-F_FACT2_UV)	
#define F_FACT1_V_R                 (Float)0.0125 /*correction 25/09/07 for step by 6, was 273 for step by 4, because problem attenuate_lin with 15 ms*/
#define F_FACT2_V_R                 (Float)0.0125 /*32768/80*/
#define F_FACT3_V_R                 (Float)0.0125
#define F_FACT2P_V_R                (F_FACT2_V_R-F_FACT1_V_R)	
#define F_FACT3P_V_R                (F_FACT3_V_R-F_FACT2_V_R)	


/* size of higher-band signal buffer */
#define LEN_HB_MEM                160
#define LEN_HB_MEM_MLF            (LEN_HB_MEM - L_FRAME_NB)

/**************
 * PLC states *
 **************/
typedef struct _G722PLC_STATE_FLT
{
  Short    s_prev_bfi; /* bad frame indicator of previous frame */

  /* signal buffers */
  Float   *f_mem_speech;     /* lower-band speech buffer */
  Float   *f_mem_exc;        /* past LPC residual */
  Float   *f_mem_speech_hb;  /* higher-band speech buffer */

  /* analysis results (signal class, LPC coefficients, pitch delay) */
  Short    s_clas;  /* unvoiced, weakly voiced, voiced */
  Short    s_t0;    /* pitch delay */
  Short    s_t0p2;     /* constant*/

  /* variables for crossfade */
  Short    s_count_crossfade; /* counter for cross-fading (number of samples) */
  Float    f_crossfade_buf[CROSSFADELEN];

  /* variables for DC remove filter in higher band */
  Float    f_mem_hpf_in;
  Float    f_mem_hpf_out;

  /* variables for synthesis attenuation */
  Short   s_count_att;    /* counter for lower-band attenuation (number of samples) */
  Short   s_count_att_hb; /* counter for higher-band attenuation (number of samples) */
  Short   s_inc_att;      /* increment for counter update */
  Float   f_fact1;
  Float   f_fact2p;
  Float   f_fact3p;
  Float   f_weight_lb;
  Float   f_weight_hb;

  /* coefficient of ARMA predictive filter A(z)/B(z) of G.722 */
  Float   *f_a;     /* LPC coefficients */
  Float   *f_mem_syn;        /* past synthesis */
} G722PLC_STATE_FLT;

void*  G722PLC_init_flt(void);
void G722PLC_conceal_flt(void * plc_state, Short* outcode, g722_state *decoder);

Float G722PLC_hp_flt(Float *x1, Float* y1, Float signal,
					const Float *G722PLC_b_hp, const Float *G722PLC_a_hp);
void   G722PLC_clear_flt(void * state);


/**************
 * PLC tables *
 **************/

extern const Float f_G722PLC_lag[ORD_LPC];
extern const Float f_G722PLC_lpc_win_80[80];
extern const Float f_G722PLC_fir_lp[FEC_L_FIR_FILTER_LTP];
extern const Float f_G722PLC_b_hp[2];
extern const Float f_G722PLC_a_hp[2];
extern const Float f_G722PLC_b_hp156[2];
extern const Float f_G722PLC_a_hp156[2];
extern const Float f_G722PLC_gamma_az[9];

/****************
 * PLC routines *
 ****************/

#endif
