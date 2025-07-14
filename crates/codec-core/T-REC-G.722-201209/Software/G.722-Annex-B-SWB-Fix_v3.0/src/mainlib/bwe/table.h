/* ITU G.722 3rd Edition (2012-09) */

/*--------------------------------------------------------------------------
 ITU-T Annex B (ex G.722-SWB) Source Code
 Software Release 1.00 (2010-09)
 (C) 2010 France Telecom, Huawei Technologies, NTT, VoiceAge Corp.
--------------------------------------------------------------------------*/

#ifndef TABLE_H
#define TABLE_H

#include "stl.h"

extern Word16 CodeBookH[];
extern Word16 scodebookL[];

extern Word16 tEnv_weight[];

/* for 4-8kHz postprocess */
extern Word16 pst_j1[];
extern Word16 pst_j2[];
extern Word16 pst_fw1[];
extern Word16 pst_fw2[];
extern Word16 pst_sumw1[];
extern Word16 pst_j11[];
extern Word16 pst_j22[];
extern Word16 pst_fw11[];
extern Word16 pst_fw22[];
extern Word16 pst_sumw2[];
extern Word16 sub_8192_pst_fw2[];
extern Word16 sub_16384_pst_fw22[];


#endif
