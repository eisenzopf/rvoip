/* ITU G.722 3rd Edition (2012-09) */

/*--------------------------------------------------------------------------
 ITU-T G.722 Annex C (ex G.722-SWB-Float) Source Code
 Software Release 1.01 (2012-07)
 (C) 2012 France Telecom, Huawei Technologies, VoiceAge Corp., NTT.
--------------------------------------------------------------------------*/

#ifndef LSBCOD_NS_H
#define LSBCOD_NS_H

#include "ns.h"
#include "funcg722.h"

void fl_lsbcod_buf_ns(const Short sigin[], Short code0[], g722_state *work1, fl_noiseshaping_state *work2, Short mode, Short local_mode);

/**************
 *     tables *
 **************/
extern const Short   code_mask[4];

#endif
