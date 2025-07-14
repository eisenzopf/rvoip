/* ITU G.722 3rd Edition (2012-09) */

/*--------------------------------------------------------------------------
 ITU-T Annex B (ex G.722-SWB) Source Code
 Software Release 1.00 (2010-09)
 (C) 2010 France Telecom, Huawei Technologies, NTT, VoiceAge Corp.
--------------------------------------------------------------------------*/

#ifndef __BWE_MDCT_TABLE_H__
#define __BWE_MDCT_TABLE_H__

#include "bwe_mdct.h"

extern Word16 sb_bound[4][2];

extern const Word16 pmag_cbk[];

extern Word16 ncb_alloc[MAX_HB_ENH_BITS+1];

extern Word16 sg2[MAX_HB_ENH_BITS+1][MAX_NCB_GAIN];
extern Word16 sge[MAX_HB_ENH_BITS+1][MAX_NCB_GAIN];

/***********************************************/
/* MDCT window, Q14 */
extern const Word16 MDCT_h_swb[];
/* Sine table for MDCT and iMDCT, Q15 */
extern const Word16 MDCT_wsin_swb[];
/* Table for complex post-multiplication in MDCT (real part), Q21 */
extern const Word16 MDCT_wetr_swb[];
/* Table for complex post-multiplication in MDCT (imaginary part), Q21 */
extern const Word16 MDCT_weti_swb[];

/* Index mapping table for Good-Thomas FFT */
extern const Word16 MDCT_tab_map_swb[];

/* Index mapping table for Good-Thomas FFT */
extern const Word16 MDCT_tab_map2_swb[];

/* Table for Good-Thomas FFT */
extern const Word16 MDCT_tab_rev_ipp_swb[];
/* Table for Good-Thomas FFT */
extern const Word16 MDCT_tab_rev_i_swb[];

/* FFT twiddle factors (cosine part), Q15 */
extern const Word16 MDCT_rw1_tbl_swb[];
/* FFT twiddle factors (sine part), Q15 */
extern const Word16 MDCT_rw2_tbl_swb[];
/* Cosine table for FFT, Q15 */
extern const Word16 MDCT_xcos_swb[];
/* Sine table for FFT, Q15 */
extern const Word16 MDCT_xsin_swb[];
/* Table for complex pre-multiplication in iMDCT (real part), Q14 */
extern const Word16 MDCT_wetrm1_swb[];
/* Table for complex pre-multiplication in iMDCT (imaginary part), Q14 */
extern const Word16 MDCT_wetim1_swb[];
/***********************************************/


extern Word16 sgain[MAX_HB_ENH_BITS+1][MAX_NCB_GAIN];

extern Word16    tabpow[];

#endif	/* __BWE_MDCT_TABLE_H__ */
