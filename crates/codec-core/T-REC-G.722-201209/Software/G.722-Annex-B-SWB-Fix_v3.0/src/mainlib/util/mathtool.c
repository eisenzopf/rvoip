/* ITU G.722 3rd Edition (2012-09) */

/*--------------------------------------------------------------------------
 ITU-T Annex B (ex G.722-SWB) Source Code
 Software Release 1.00 (2010-09)
 (C) 2010 France Telecom, Huawei Technologies, NTT, VoiceAge Corp.
--------------------------------------------------------------------------*/

#include "pcmswb_common.h"
#include "mathtool.h"

/*****************************/
#ifdef DYN_RAM_CNT
#include "dyn_ram_cnt.h"
#endif
/*****************************/

#define Message(x)

/*___________________________________________________________________________
 |                                                                           |
 |   Function Name: SqrtI31                                                  |
 |       Squareroot of a 32-bit variable                                     |
 |       Input and output are assumed to be in Q31.                          |
 |       The input Q must be odd.                                            |
 |       The output Q is (31-( 31-inQ )/2).                                  |
 |   Return value:                                                           |
 |       1 for OK, 0 for NG                                                    |
 |___________________________________________________________________________|
*/
Word16 SqrtI31(
  Word32 lInput,   /* (i): Input data, Q31 */
  Word32 *plOutput /* (o): Output data, Q31 */
)
{
  Word16 sLow;
  Word16 nExp;
  Word16 nExp2;
  Word16 sDiff;
  Word16 index;
  Word32 lAcc;

/*****************************/
#ifdef DYN_RAM_CNT
  {
    UWord32 ssize = 0;
    ssize += (UWord32) (5 * SIZE_Word16);
    ssize += (UWord32) (1 * SIZE_Word32);
    DYN_RAM_PUSH(ssize, "dummy");
  }
#endif
/*****************************/

  if ( lInput < 0 )
  {
    Message("ERROR : SqrtI() : input data < 0\n");
/*****************************/
#ifdef DYN_RAM_CNT
    DYN_RAM_POP();
#endif
/*****************************/
    return 0;
  }
  else if ( lInput == 0 )
  {
    *plOutput = 0;
/*****************************/
#ifdef DYN_RAM_CNT
    DYN_RAM_POP();
#endif
/*****************************/
    return 1;
  }

  nExp = norm_l( lInput );

  nExp2 = shr( (Word16)nExp, 1 );
  nExp = shl( (Word16)nExp2, 1 );

  lAcc = L_shl( lInput, (Word16)nExp ); /* Q(31+exp) = Q31 << exp */

  lAcc = L_shr( lAcc, 9 ); /* 30 ~ 25 bit -> index */

  index = extract_h( lAcc );

  /* sLow = extract_l( (Word32)0x0000ffff & L_shr( L_msu( lAcc, shl( index, 1 ), 16384 ), 1 ) ); */
  lAcc = L_sub( lAcc, L_deposit_h( (Word16)index ) );
  sLow = extract_h( L_shl( lAcc, 15 ) );

  lAcc = L_deposit_h( table_sqrt_w[index-16] );

  sDiff = sub( table_sqrt_w[index-16], table_sqrt_w[index+1-16] );

  lAcc = L_msu( lAcc, sLow, sDiff );

  *plOutput = L_shr( lAcc, (Word16)nExp2 );

/*****************************/
#ifdef DYN_RAM_CNT
  DYN_RAM_POP();
#endif
/*****************************/

  return 1;
}

/*___________________________________________________________________________
 |                                                                           |
 |   Function Name : Isqrt_n                                                 |
 |                                                                           |
 |       Compute 1/sqrt(value).                                              |
 |       if value is negative or zero, result is 1 (frac=7fffffff, exp=0).   |
 |---------------------------------------------------------------------------|
 |  Algorithm:                                                               |
 |                                                                           |
 |   The function 1/sqrt(value) is approximated by a table and linear        |
 |   interpolation.                                                          |
 |                                                                           |
 |   1- If exponant is odd then shift fraction right once.                   |
 |   2- exponant = -((exponant-1)>>1)                                        |
 |   3- i = bit25-b30 of fraction, 16 <= i <= 63 ->because of normalization. |
 |   4- a = bit10-b24                                                        |
 |   5- i -=16                                                               |
 |   6- fraction = table[i]<<16 - (table[i] - table[i+1]) * a * 2            |
 |___________________________________________________________________________|
*/
void Isqrt_n(
  Word32 * frac,  /* (i/o) Q31: normalized value (1.0 < frac <= 0.5) */
  Word16 * exp    /* (i/o)    : exponent (value = frac x 2^exponent) */
)
{
  Word16  i, a, tmp;
  Word32  L_tmp;

/*****************************/
#ifdef DYN_RAM_CNT
  {
    UWord32 ssize = 0;
    ssize += (UWord32) (3 * SIZE_Word16);
    ssize += (UWord32) (1 * SIZE_Word32);
    DYN_RAM_PUSH(ssize, "dummy");
  }
#endif
/*****************************/

  L_tmp = L_shr(*frac, s_and(*exp, 1));

  /* 1) -16384 to shift left and change sign                 */
  /* 2) 32768 to Add 1 to Exponent like it was divided by 2  */
  /* 3) We let the mac_r add another 0.5 because it imitates */
  /*    the behavior of shr on negative number that should   */
  /*    not be rounded towards negative infinity.            */
  *exp = mac_r(32768, *exp, -16384);     move16();

  L_tmp = L_shr(L_tmp, 9);
  a = extract_l(L_tmp);                  /* Extract b10-b24 */
  a = lshr(a, 1);

  i = mac_r(L_tmp, -16*2-1, 16384);      /* Extract b25-b31 minus 16 */

  L_tmp = L_mult(table_isqrt[i], -32768);   /* table[i] << 16         */
  tmp = sub(table_isqrt[i], table_isqrt[i + 1]);      /* table[i] - table[i+1]) */
  move32();
  *frac = L_mac(L_tmp, tmp, a);          /* frac -=  tmp*a*2       */

/*****************************/
#ifdef DYN_RAM_CNT
  DYN_RAM_POP();
#endif
/*****************************/
}

/*___________________________________________________________________________
 |                                                                           |
 |   Function Name : Inv_sqrt                                                |
 |                                                                           |
 |       Compute 1/sqrt(L_x).                                                |
 |       L_x is positive.                                                    |
 |                                                                           |
 |       if L_x is negative or zero, result is 1 (3fff ffff).                |
 |---------------------------------------------------------------------------|
 |  Algorithm:                                                               |
 |                                                                           |
 |   The function 1/sqrt(L_x) is approximated by a table and linear          |
 |   interpolation.                                                          |
 |                                                                           |
 |   1- Normalization of L_x.                                                |
 |   2- If (30-exponent) is even then shift right once.                      |
 |   3- exponent = (30-exponent)/2  +1                                       |
 |   4- i = bit25-b31 of L_x,    16 <= i <= 63  ->because of normalization.  |
 |   5- a = bit10-b24                                                        |
 |   6- i -=16                                                               |
 |   7- L_y = tabsqr[i]<<16 - (tabsqr[i] - tabsqr[i+1]) * a * 2              |
 |   8- L_y >>= exponent                                                     |
 |___________________________________________________________________________|
*/
Word32 Inv_sqrt( /* (o) Q30 : output value   (range: 0<=val<1)           */
  Word32  L_x    /* (i) Q0  : input value    (range: 0<=val<=7fffffff)   */
)
{
  Word32  L_y;
  Word16  exp;

/*****************************/
#ifdef DYN_RAM_CNT
  {
    UWord32 ssize = 0;
    ssize += (UWord32) SIZE_Word16;
    ssize += (UWord32) SIZE_Word32;
    DYN_RAM_PUSH(ssize, "dummy");
  }
#endif
/*****************************/

  if (L_x <= (Word32) 0) 
  {
/*****************************/
#ifdef DYN_RAM_CNT
    DYN_RAM_POP();
#endif
/*****************************/
    return (Word32)0x3fffffffL;
  }

  L_y = norm_l_L_shl(&exp, L_x);

  exp = sub (31, exp);

  Isqrt_n (&L_y, &exp);

  L_y = L_shr (L_y, sub(1,exp));

/*****************************/
#ifdef DYN_RAM_CNT
  DYN_RAM_POP();
#endif
/*****************************/

  return L_y;
}
