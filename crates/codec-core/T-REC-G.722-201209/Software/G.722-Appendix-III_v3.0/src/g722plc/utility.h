/* ITU G.722 3rd Edition (2012-09) */

/* ITU-T G.722 Appendix III                                                      */
/* Version:       1.0                                                            */
/* Revision Date: Nov.02, 2006                                                   */

/*
  ITU-T G.722 Appendix III ANSI-C Source Code
  Copyright (c)  Broadcom Corporation 2006.
*/

#include "typedef.h"

extern void W16copy(Word16 *y, Word16 *x, int size);
extern void W16zero(Word16 *x, int size);
extern void W32copy(Word32 *y, Word32 *x, int size);

Word32 Mpy_32(Word16 hi1, Word16 lo1, Word16 hi2, Word16 lo2);
Word32 Mpy_32_16(Word16 hi, Word16 lo, Word16 n);
Word32 Div_32(Word32 L_num, Word16 denom_hi, Word16 denom_lo);
void L_Extract(Word32 L_32, Word16 *hi, Word16 *lo);
Word32 L_Comp(Word16 hi, Word16 lo);
