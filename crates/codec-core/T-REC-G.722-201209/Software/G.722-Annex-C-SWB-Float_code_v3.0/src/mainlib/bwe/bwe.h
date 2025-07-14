/* ITU G.722 3rd Edition (2012-09) */

/*--------------------------------------------------------------------------
 ITU-T G.722 Annex C (ex G.722-SWB-Float) Source Code
 Software Release 1.01 (2012-07)
 (C) 2012 France Telecom, Huawei Technologies, VoiceAge Corp., NTT.
--------------------------------------------------------------------------*/

#ifndef BWE_H
#define BWE_H

#include "pcmswb_common.h"

/*------------------------------------------------------------------------*
 * Defines
 *------------------------------------------------------------------------*/

#define EPS      1.0e-3f 
#define FAC_LOG2 3.321928095f 
#define Fabs(x)  ((x)<0?-(x):(x))  
#define INV_TRANSI_FENV_EXPAND 0.2f  

#define SWB_NORMAL_FENV        8
#define SWB_TRANSI_FENV        4
#define SWB_TRANSI_FENV_WIDTH  16
#define SWB_TENV               4
#define SWB_F_WIDTH            64
#define ZERO_SWB               20
#define SWB_T_WIDTH            80
#define SWB_TENV_WIDTH         20
#define TRANSIENT              3
#define HARMONIC               2
#define NORMAL                 0
#define NUM_FRAME              3
#define TRANSI_FENV_EXPAND     5
#define VQ_FENV_SIZE           64
#define VQ_FENV_DIM            4
#define NUM_SHARP              10
#define SHARP_WIDTH            6
#define FENV_WIDTH             (SWB_F_WIDTH / SWB_NORMAL_FENV)	
#define SWB_F_WIDTH_HALF       (SWB_F_WIDTH/2)
#define NBITS_MODE_R1SM_TOTLE  40
#define NBITS_MODE_R1SM_BWE    21
#define NBITS_MODE_R1SM_WBE    (NBITS_MODE_R1SM_TOTLE - NBITS_MODE_R1SM_BWE)	
#define NBytesPerFrame_R1SM    5
#define NUM_FENV_VECT          2
#define NUM_FENV_CODEBOOK      2
#define SUB_SWB_T_WIDTH        (SWB_T_WIDTH/4) 	
#define HALF_SUB_SWB_T_WIDTH   (SUB_SWB_T_WIDTH/2)	
#define HALF_SUB_SWB_T_WIDTH_1 HALF_SUB_SWB_T_WIDTH-1
#define HALF_SUB_SWB_T_WIDTH_2 HALF_SUB_SWB_T_WIDTH-2
#define HALF_SUB_SWB_T_WIDTH_3 HALF_SUB_SWB_T_WIDTH-3
#define WB_POSTPROCESS_WIDTH  36
#define SWB_NORMAL_FENV_HALF   (SWB_NORMAL_FENV/2)	
#define NUM_PRE_SWB_TENV      ((NUM_FRAME-1)*SWB_TENV)	
#define NORMAL_FENV_HALVE     (SWB_NORMAL_FENV/2)	
#define ENERGY_WB             45

typedef struct {
	Short preMode;
	Float preGain;
	Float fIn[SWB_T_WIDTH];
	Float stEnvPre[(NUM_FRAME - 1) * SWB_TENV];	
	Short modeCount;
	Float log_rms_fix_pre[NUM_PRE_SWB_TENV];
	Float enerEnvPre[NUM_FRAME - 1];	
	Float pre_sy[SWB_T_WIDTH];
} BWE_state_enc;  

typedef struct {    /* used in decoder only */
	Float pre_tEnv;
	Float fpre_wb[SWB_T_WIDTH];
	Float fPrev[L_FRAME_WB];
	Float fCurSave[L_FRAME_WB];
	Float fPrev_wb[L_FRAME_WB];
	Float fCurSave_wb[L_FRAME_WB];
	Float pre_fEnv[10];
	Float tPre[HALF_SUB_SWB_T_WIDTH];	
	Short norm_pre;
	Short norm_pre_wb;
	Short pre_mode;
	Float attenu2;
	Float prev_enerL;
	Float spGain_sm[WB_POSTPROCESS_WIDTH]; 
	Short modeCount;
	Short Seed;
} BWE_state_dec; 

/*------------------------------------------------------------------------*
 * Prototypes
 *------------------------------------------------------------------------*/
Short bwe_encode_reset (void *work);
void*  bwe_encode_const (void);
void   bwe_encode_dest (void *work);
Short Icalc_tEnv(
					Float *sy,              /* (o)   current SWB high band signal    */
					Float * rms,            /* (o)    log2 of the temporal envelope  */
					Short * transient,
					int preMode,
					void* work
					);
Short bwe_enc(
					Float          fBufin[],           /* (i): Input super-higher-band signal */
					unsigned short **pBit,             /* (o): Output bitstream               */
					void           *work,        /* (i/o): Pointer to work space        */
					Float          *tEnv,              /* (i) */
					Short          transi,
					Short          *cod_Mode,
					Float          *f_Fenv_SWB,        /* (o) */
					Float          *fSpectrum,         /* (o) */
					Short          *index_g,
					Short          T_modify_flag,
					Float          fEnv_unq[]          /* (o) */
					);
Short bwe_dec_update( /*to maintain mid-band post-processing memories up to date in case of WB frame*/
					Float  	       *fy_low,    	       /* (i): Input lower-band WB signal */
					void           *work               /* (i/o): Pointer to work space        */
					);
Short bwe_decode_reset (void *work);
void*  bwe_decode_const (void);
void   bwe_decode_dest (void *work);
Short bwe_dec_freqcoef( 
					unsigned short **pBit,             /* (i): Input bitstream                */
					Float  *fy_low,    	               /* (i): Input lower-band WB signal */
					void   *work,                      /* (i/o): Pointer to work space        */
					Short  *sig_Mode,
					Float  *f_sTenv_SWB,               /* (o) */ 
					Float  *f_scoef_SWB,
					Short  *index_g,
					Float  *f_sFenv_SVQ,               /* (o): decoded spectral envelope with no postprocess. */
					Short  ploss_status,
					Short  bit_switch_flag,
					Short  prev_bit_switch_flag
					);
Short bwe_dec_timepos(
					int sig_Mode,
					Float *Tenv_SWB,
					Float *coef_SWB,
					Float *fy_hi,       /* (o): Output higher-band signal */
					void  *work,        /* (i/o): Pointer to work space        */
					int erasure,
					int T_modify_flag
					);
#endif
