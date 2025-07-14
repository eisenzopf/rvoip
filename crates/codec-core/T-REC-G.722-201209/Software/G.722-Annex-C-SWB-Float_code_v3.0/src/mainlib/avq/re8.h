/* ITU G.722 3rd Edition (2012-09) */

/*--------------------------------------------------------------------------
 ITU-T G.722 Annex C (ex G.722-SWB-Float) Source Code
 Software Release 1.01 (2012-07)
 (C) 2012 France Telecom, Huawei Technologies, VoiceAge Corp., NTT.
--------------------------------------------------------------------------*/

#ifndef RE8_H
#define RE8_H

#include "floatutil.h"

/* RE8 lattice quantiser functions in re8_*.c */

void RE8_ppv(Float x[], Short y[]);
void RE8_k2y_flt( Short *k, Short m, Short *y );
void RE8_vor( Short y[], Short *n, Short k[], Short c[], Short *ka );

void s_re8_compute_base_index( const Short *x, const Short ka, unsigned short *I );


void re8_decode_base_index_flt(Short n, unsigned short I, Short *x);

#endif

