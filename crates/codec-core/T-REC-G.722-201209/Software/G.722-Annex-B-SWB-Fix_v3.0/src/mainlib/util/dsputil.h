/* ITU G.722 3rd Edition (2012-09) */

/*--------------------------------------------------------------------------
 ITU-T Annex B (ex G.722-SWB) Source Code
 Software Release 1.00 (2010-09)
 (C) 2010 France Telecom, Huawei Technologies, NTT, VoiceAge Corp.
--------------------------------------------------------------------------*/

#ifndef DSPUTIL_H
#define DSPUTIL_H

#ifndef _STDLIB_H_
#include <stdlib.h>
#endif
#ifndef _STL_H
#include "stl.h"
#endif

#define a_mac() mac_r(0L,0,0)       /* Addressing MAC operator */

void   zero16( Word16 n, Word16 *xx_16 );
void   mov16( Word16 n, Word16 *xx_16, Word16 *yy_16 );
void   mov16_bwd( Word16 n, Word16  *sx, Word16  *sy);
Word16 bound( Word16 x, Word16  x_min, Word16  x_max);
Word16 MaxAbsArray(Word16 n, Word16 *sx, Word16 *ind);
Word32 L_mac0_Array(Word16 n, Word16 *sx, Word16 *sy);
Word32 L_mac_Array(Word16 n, Word16 *sx, Word16 *sy);
Word16 Exp16Array(Word16 n, Word16  *sx);
Word32 Sum_vect_E(const Word16 *vec, const Word16 lvec);
Word16 MaxArray(Word16 n, Word16 *sx, Word16 *ind);
Word32 L_add_Array(Word16 n, Word16 *sx);
void   const16(Word16 n, Word16 con, Word16 *sx);
void   L_mac_shr(Word16 len, Word32 *L_temp, Word16 b, Word16 *spit);
void   mov16_ext(Word16 n, Word16 *sx, Word16 m, Word16 *sy, Word16 l);
void   abs_array(Word16 *a, Word16 *b, Word16 L);
void   array_oper(Word16 n, Word16 b, Word16 *sx, Word16 *sy, Word16 (*ptf)(Word16, Word16));
void   array_oper_ext(Word16 n, Word16 b, Word16 *sx, Word16 m, Word16 *sy, Word16 l, Word16 (*ptf)(Word16, Word16));
Word16 extract_h_L_shl(Word32 t32, Word16 b);
Word16 extract_h_L_shr_sub(Word32 L_tmp, Word16 a, Word16 b);
Word16 round_fx_L_shl_L_mult(Word16 a, Word16 b, Word16 c);
Word16 round_fx_L_shl(Word32 a, Word16 b);
Word32 norm_l_L_shl(Word16 *exp_den, Word32 L_en);
Word16 round_fx_L_shr_L_mult(Word16 a, Word16 b, Word16 c);
Word16 extract_l_L_shr(Word32 a, Word16 b);
Word32 L_abs_L_deposit_l(Word16 a);
void   FOR_L_mult_L_shr_L_add(Word16 a, Word16 *spMDCT_wb, Word16 b, Word32* L_temp1, Word32* L_temp);

#include "mathtool.h"

#endif
