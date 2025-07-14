/* ITU G.722 3rd Edition (2012-09) */

/*--------------------------------------------------------------------------
 ITU-T G.722 Annex C (ex G.722-SWB-Float) Source Code
 Software Release 1.01 (2012-07)
 (C) 2012 France Telecom, Huawei Technologies, VoiceAge Corp., NTT.
--------------------------------------------------------------------------*/

#ifndef __BWE_MDCT_TABLE_H__
#define __BWE_MDCT_TABLE_H__

#include "bwe_mdct.h"


extern const Float MDCT_xcos_swbf[25];
extern const Float MDCT_xsin_swbf[25];
extern const Float MDCT_rw1_tbl_swbf[MDCT2_SBARYSZ];  
extern const Float MDCT_rw2_tbl_swbf[MDCT2_SBARYSZ];  

extern const Short MDCT_tab_map_swbs[MDCT2_NP*MDCT2_NPP];
extern const Short MDCT_tab_map2_swbs[MDCT2_NP*MDCT2_NPP];
extern const Short MDCT_tab_rev_ipp_swbs[MDCT2_NB_REV];
extern const Short MDCT_tab_rev_i_swbs[MDCT2_NB_REV];

extern const Float MDCT_h_swbf[MDCT2_L_WIN2];
extern const Float MDCT_wsin_swbf[MDCT2_L_WIN4+1];
extern const Float MDCT_wetr_swbf[MDCT2_L_WIN4];
extern const Float MDCT_weti_swbf[MDCT2_L_WIN4];
extern const Float MDCT_rw1_tbl_swbf[MDCT2_SBARYSZ];  
extern const Float MDCT_rw2_tbl_swbf[MDCT2_SBARYSZ];  
extern const Float MDCT_wetrm1_swbf[MDCT2_L_WIN4];
extern const Float MDCT_wetim1_swbf[MDCT2_L_WIN4];

extern const Short MDCT_tab_rev_ipp_swbs[MDCT2_NB_REV];
extern const Short MDCT_tab_rev_i_swbs[MDCT2_NB_REV];
extern const Short MDCT_tab_map_swbs[MDCT2_NP*MDCT2_NPP];
extern const Short MDCT_tab_map2_swbs[MDCT2_NP*MDCT2_NPP];


#endif	/* __BWE_MDCT_TABLE_H__ */
