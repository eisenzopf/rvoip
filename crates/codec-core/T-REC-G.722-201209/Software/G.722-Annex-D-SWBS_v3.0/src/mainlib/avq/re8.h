/* ITU G.722 3rd Edition (2012-09) */

/*--------------------------------------------------------------------------
 ITU-T Annex B (ex G.722-SWB) Source Code
 Software Release 1.00 (2010-09)
 (C) 2010 France Telecom, Huawei Technologies, NTT, VoiceAge Corp.
--------------------------------------------------------------------------*/

#ifndef RE8_H
#define RE8_H

#include "stl.h"

/* RE8 lattice quantiser functions in re8_*.c */

void RE8_PPV( Word32 x[], Word16 y[] );
void RE8_k2y( Word16 *k, Word16 m, Word16 *y );
void RE8_Vor( Word16 y[], Word16 *n, Word16 k[], Word16 c[], Word16 *ka );

void re8_compute_base_index( const Word16 *x, const Word16 ka, UWord16 *I );
void re8_decode_base_index(Word16 n, UWord16 I, Word16 *x);

#endif

