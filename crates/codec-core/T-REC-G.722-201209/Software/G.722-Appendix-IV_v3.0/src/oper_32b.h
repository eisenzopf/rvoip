/* ITU G.722 3rd Edition (2012-09) */

/* ITU-T G722 PLC Appendix IV - Reference C code for fixed-point implementation */
/* Version:       1.2                                                           */
/* Revision Date: Jul 3, 2007                                                   */

/*
   ITU-T G.722 PLC Appendix IV   ANSI-C Source Code
   Copyright (c) 2006-2007
   France Telecom
*/

#ifndef __G729EV_MAIN_OPER_32B_H__
#define __G729EV_MAIN_OPER_32B_H__

#include "stl.h"

/* Double precision operations */

void      L_Extract(Word32 L_32, Word16 * hi, Word16 * lo);
Word32    L_Comp(Word16 hi, Word16 lo);
Word32    Mpy_32(Word16 hi1, Word16 lo1, Word16 hi2, Word16 lo2);
Word32    Mpy_32_16(Word16 hi, Word16 lo, Word16 n);
Word32    Div_32(Word32 L_num, Word16 denom_hi, Word16 denom_lo);

#endif /* __G729EV_MAIN_OPER_32B_H__ */
