/* ITU G.722 3rd Edition (2012-09) */

/* ITU-T G.722 Appendix III                                                      */
/* Version:       1.0                                                            */
/* Revision Date: Nov.02, 2006                                                   */

/*
  ITU-T G.722 Appendix III ANSI-C Source Code
  Copyright (c)  Broadcom Corporation 2006.
*/

#include "typedef.h"
#include "stl.h"
#include "table.h"
#include "utility.h"

/*___________________________________________________________________________
 |                                                                           |
 |   Function Name : Log2()                                                  |
 |                                                                           |
 |       Compute log2(L_x).                                                  |
 |       L_x is positive.                                                    |
 |                                                                           |
 |       if L_x is negative or zero, result is 0.                            |
 |---------------------------------------------------------------------------|
 |  Algorithm:                                                               |
 |                                                                           |
 |   The function Log2(L_x) is approximated by a table and linear            |
 |   interpolation.                                                          |
 |                                                                           |
 |   1- Normalization of L_x.                                                |
 |   2- exponent = 30-exponent                                               |
 |   3- i = bit25-b31 of L_x,    32 <= i <= 63  ->because of normalization.  |
 |   4- a = bit10-b24                                                        |
 |   5- i -=32                                                               |
 |   6- fraction = tablog[i]<<16 - (tablog[i] - tablog[i+1]) * a * 2            |
 |___________________________________________________________________________|
*/

void Log2(
  Word32 L_x,       /* (i) Q0 : input value                                 */
  Word16 *exponent, /* (o) Q0 : Integer part of Log2.   (range: 0<=val<=30) */
  Word16 *fraction  /* (o) Q15: Fractional  part of Log2. (range: 0<=val<1) */
)
{
  Word16 exp, i, a, tmp;
  Word32 L_y;

  IF( L_x <= (Word32)0 )
  {
    *exponent = 0;
    *fraction = 0;
#if WMOPS
    move16();move16();
#endif
    return;
  }

  exp = norm_l(L_x);
  L_x = L_shl(L_x, exp );               /* L_x is normalized */

  *exponent = sub(30, exp);

  L_x = L_shr(L_x, 9);
  i   = extract_h(L_x);                 /* Extract b25-b31 */
  L_x = L_shr(L_x, 1);
  a   = extract_l(L_x);                 /* Extract b10-b24 of fraction */
  a   = s_and(a, (Word16)0x7fff);

  i   = sub(i, 32);

  L_y = L_deposit_h(tablog[i]);          /* tablog[i] << 16        */
  tmp = sub(tablog[i], tablog[add(i, 1)]);      /* tablog[i] - tablog[i+1] */
  L_y = L_msu(L_y, tmp, a);             /* L_y -= tmp*a*2        */

  *fraction = extract_h( L_y);
#if WMOPS
  move16();move16();
#endif
  return;
}
