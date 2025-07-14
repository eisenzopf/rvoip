/* ITU G.722 3rd Edition (2012-09) */

/*--------------------------------------------------------------------------
 ITU-T Annex B (ex G.722-SWB) Source Code
 Software Release 1.00 (2010-09)
 (C) 2010 France Telecom, Huawei Technologies, NTT, VoiceAge Corp.
--------------------------------------------------------------------------*/

#ifndef __BWE_MDCT_H__
#define __BWE_MDCT_H__

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

#define MDCT2_SB       10                   /* constants for spectral envelope coding in PCMSWB coder */

#define MAX_HB_ENH_BITS 3                   /* constants for bit allocation and VQ */
#define MAX_NCB_GAIN   (1<<MAX_HB_ENH_BITS) /* constants for bit allocation and VQ */

/*------------------------------------------------------------------------*
 * Prototypes
 *------------------------------------------------------------------------*/
void bwe_mdct (Word16 * mem, Word16 * input, Word16 * ykr, Word16 *norm_shift);
void PCMSWB_TDAC_inv_mdct (Word16 * xr, Word16 * ykq, Word16 * ycim1, Word16 norm_shift, Word16 * norm_pre, Word16 loss_flag, Word16 * cur_save);

#endif  /*__GPCMSWB_TDAC_MDCT_H__ */
