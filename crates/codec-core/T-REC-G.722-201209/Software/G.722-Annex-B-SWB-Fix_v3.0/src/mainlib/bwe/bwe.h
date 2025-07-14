/* ITU G.722 3rd Edition (2012-09) */

/*--------------------------------------------------------------------------
 ITU-T Annex B (ex G.722-SWB) Source Code
 Software Release 1.00 (2010-09)
 (C) 2010 France Telecom, Huawei Technologies, NTT, VoiceAge Corp.
--------------------------------------------------------------------------*/

#ifndef BWE_H
#define BWE_H

#include "pcmswb_common.h"

/*------------------------------------------------------------------------*
 * Defines
 *------------------------------------------------------------------------*/
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
  Word16 preMode;
  Word16 preGain;
  Word16 sIn[SWB_T_WIDTH];
  Word32 stEnvPre[(NUM_FRAME - 1) * SWB_TENV];	
  Word16 modeCount;
  Word16 log_rms_fix_pre[NUM_PRE_SWB_TENV];
  Word32 enerEnvPre[NUM_FRAME - 1];	
  Word16 pre_sy[SWB_T_WIDTH];
} BWE_state_enc;

typedef struct {    /* used in decoder only */
  Word16 pre_tEnv;
  Word16 pre_wb[SWB_T_WIDTH];
  Word16 pre_fEnv[10];
  Word16 tPre[HALF_SUB_SWB_T_WIDTH];
  Word16 sPrev[L_FRAME_WB];
  Word16 sCurSave[L_FRAME_WB];
  Word16 norm_pre;
  Word16 sPrev_wb[L_FRAME_WB];
  Word16 sCurSave_wb[L_FRAME_WB];
  Word16 norm_pre_wb;
  Word16 pre_mode;
  Word16 sattenu2;
  Word16 pre_coef_SWBQ;
  Word16 prev_senerL;
  Word16 spGain_sm[WB_POSTPROCESS_WIDTH];
  Word16 modeCount;
  Word32 Seed;
} BWE_state_dec;

/*------------------------------------------------------------------------*
 * Prototypes
 *------------------------------------------------------------------------*/
Word32 bwe_dec_update( 
                      Word16		 *y_low,    	   /* (i): Input lower-band WB signal */
                      void           *work            /* (i/o): Pointer to work space        */
                      );
Word16 bwe_decode_reset (void *work);
Word16 bwe_encode_reset (void *work);



void*  bwe_encode_const (void);
void   bwe_encode_dest (void *work);

void*  bwe_decode_const (void);
void   bwe_decode_dest (void *work);

Word16
Icalc_tEnv( Word16 *sy,       /* (o)   current SWB high band signal    */
           Word16 * rms,     /* (o)    log2 of the temporal envelope  */
           Word16 * transient,
           Word16 preMode
           , void* work    
           );


Word16 bwe_enc( Word16         sBufin[],           /* (i): Input super-higher-band signal */
               UWord16        **pBit,             /* (o): Output bitstream               */
               void           *work,              /* (i/o): Pointer to work space        */
               Word16         *stEnv,             /* (i): Q(0) */
               Word16         transi,
               Word16         *cod_Mode,
               Word16         *sfEnv,             /* (o): Q(0) */
               Word16         *sfSpectrum,        /* (o): Q(0) */
               Word16         *index_g,
               Word16         T_modify_flag,
               Word16         sfEnv_unq[],         /* (o): Q(12) */
               Word16         *sfSpectrumQ
               );

Word16 bwe_dec_freqcoef( UWord16 **pBit,            /* (i): Input bitstream                */
                        Word16  *y_low,    	       /* (i): Input lower-band WB signal */
                        void    *work,              /* (i/o): Pointer to work space        */
                        Word16  *sig_Mode,
                        Word16  *sTenv_SWB,         /* (o): Q(0) */ 
                        Word16  *scoef_SWB,
                        Word16  *index_g,
                        Word16  *Fenv_SVQ,          /* (o): decoded spectral envelope with no postprocess. */
                        Word16     ploss_status,
                        Word16  bit_switch_flag,
                        Word16  prev_bit_switch_flag,
                        Word16  *scoef_SWBQ
                        );

Word16 bwe_dec_timepos( Word16          sig_Mode,
                       Word16          *sTenv_SWB,  /* (i/o): Q(0) */
                       Word16          *scoef_SWB,
                       Word16          *y_hi,       /* (o): Output higher-band signal (Q0) */
                       void            *work,       /* (i/o): Pointer to work space        */
                       Word16             erasure,
                       Word16          T_modify_flag,
                       Word16          *scoef_SWBQ
                       );

#endif

