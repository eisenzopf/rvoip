/* ITU G.722 3rd Edition (2012-09) */

/*--------------------------------------------------------------------------
 ITU-T Annex B (ex G.722-SWB) Source Code
 Software Release 1.00 (2010-09)
 (C) 2010 France Telecom, Huawei Technologies, NTT, VoiceAge Corp.
--------------------------------------------------------------------------*/

#ifndef LSBCOD_NS_H
#define LSBCOD_NS_H

#include "ns.h"
#include "funcg722.h"

void lsbcod_buf_ns(const Word16 sigin[], Word16 code0[], g722_state *work1, noiseshaping_state *work2, Word16 mode, Word16 local_mode);

/**************
 *     tables *
 **************/
extern const Word16   code_mask[4];

#endif
