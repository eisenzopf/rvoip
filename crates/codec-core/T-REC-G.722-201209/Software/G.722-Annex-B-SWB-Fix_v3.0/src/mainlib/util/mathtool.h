/* ITU G.722 3rd Edition (2012-09) */

/*--------------------------------------------------------------------------
 ITU-T Annex B (ex G.722-SWB) Source Code
 Software Release 1.00 (2010-09)
 (C) 2010 France Telecom, Huawei Technologies, NTT, VoiceAge Corp.
--------------------------------------------------------------------------*/

#ifndef MATHTOOL_H
#define MATHTOOL_H

Word16 SqrtI31( Word32 input, Word32 *output );
void   Isqrt_n( Word32 *frac, Word16 *exp );
Word32 Inv_sqrt( Word32 L_x );

/* Tables */
extern const Word16 table_sqrt_w[49];
extern Word16 table_isqrt[49];

#endif
