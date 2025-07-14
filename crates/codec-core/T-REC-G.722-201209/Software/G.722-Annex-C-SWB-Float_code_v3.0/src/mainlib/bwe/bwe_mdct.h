/* ITU G.722 3rd Edition (2012-09) */

/*--------------------------------------------------------------------------
 ITU-T G.722 Annex C (ex G.722-SWB-Float) Source Code
 Software Release 1.01 (2012-07)
 (C) 2012 France Telecom, Huawei Technologies, VoiceAge Corp., NTT.
--------------------------------------------------------------------------*/

#ifndef __BWE_MDCT_H__
#define __BWE_MDCT_H__

#include "floatutil.h"
/*------------------------------------------------------------------------*
 * Defines
 *------------------------------------------------------------------------*/
#define MDCT2_L_WIN   160                   /* constants for MDCT and inverse MDCT in PCMSWB coder */
#define MDCT2_L_WIN2   80                   /* constants for MDCT and inverse MDCT in PCMSWB coder */
#define MDCT2_L_WIN4   40                   /* constants for MDCT and inverse MDCT in PCMSWB coder */
#define MDCT2_NP        5                   /* constants for MDCT and inverse MDCT in PCMSWB coder */
#define MDCT2_EXP_NPP   3                   /* constants for MDCT and inverse MDCT in PCMSWB coder */
#define MDCT2_NB_REV    2                   /* constants for MDCT and inverse MDCT in PCMSWB coder */
#define MDCT2_NPP      (1<<MDCT2_EXP_NPP)   /* constants for MDCT and inverse MDCT in PCMSWB coder */
#define MDCT2_SBARYSZ (1 << (MDCT2_EXP_NPP-1))

#define MDCT2_SB       10                   /* constants for spectral envelope coding in PCMSWB coder */

#define MAX_HB_ENH_BITS 3                   /* constants for bit allocation and VQ */
#define MAX_NCB_GAIN   (1<<MAX_HB_ENH_BITS)	/* constants for bit allocation and VQ */

/*------------------------------------------------------------------------*
 * Prototypes
 *------------------------------------------------------------------------*/
void f_bwe_mdct(
  Float * f_mem,        /* (i): old input samples    */
  Float * f_input,      /* (i): input samples        */
  Float * f_ykr,        /* (o): MDCT coefficients    */
  Short mode		   /* (i): mdct mode (0: 40-points, 1: 80-points) */
);

void f_PCMSWB_TDAC_inv_mdct(
  Float * xr,         /* (o):   output samples                     */
  Float * ykq,        /* (i):   MDCT coefficients                  */
  Float * ycim1,      /* (i):   previous MDCT memory               */
  Short   loss_flag,  /* (i):   packet-loss flag                   */
  Float * cur_save    /* (i/o): signal saving buffer               */	
);

#endif  /*__GPCMSWB_TDAC_MDCT_H__ */
