/* ITU G.722 3rd Edition (2012-09) */

/*-----------------------------------------------------------------------------------
 ITU-T G.722-SWBS / G.722 Annex D - Reference C code for fixed-point implementation          
 Version 1.0
 Copyright (c) 2012,
 Huawei Technologies
-----------------------------------------------------------------------------------*/

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

extern const Word32 L_mult_table_isqrt_m32768[49];
extern const Word16 table_isqrt_i_i1[48];
extern const Word32 L_deposit_h_table_sqrt_w[49];
extern const Word16 sub_table_sqrt_w_i_i1[48];

#endif
