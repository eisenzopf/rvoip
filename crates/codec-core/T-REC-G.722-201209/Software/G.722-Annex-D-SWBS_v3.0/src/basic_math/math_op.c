/* ITU G.722 3rd Edition (2012-09) */

/*-----------------------------------------------------------------------------------
 ITU-T G.722-SWBS / G.722 Annex D - Reference C code for fixed-point implementation          
 Version 1.0
 Copyright (c) 2012,
 Huawei Technologies
-----------------------------------------------------------------------------------*/

/*___________________________________________________________________________
 |                                                                           |
 |  This file contains mathematic operations in fixed point.                 |
 |                                                                           |
 |  Isqrt()              : inverse square root (16 bits precision).          |
 |  Pow2()               : 2^x  (16 bits precision).                         |
 |  Log2()               : log2 (16 bits precision).                         |
 |  Dot_product()        : scalar product of <x[],y[]>                       |
 |                                                                           |
 |  In this file, the values use theses representations:                     |
 |                                                                           |
 |  Word32 L_32     : standard signed 32 bits format                         |
 |  Word16 hi, lo   : L_32 = hi<<16 + lo<<1  (DPF - Double Precision Format) |
 |  Word32 frac, Word16 exp : L_32 = frac << exp-31  (normalised format)     |
 |  Word16 int, frac        : L_32 = int.frac        (fractional format)     |
 |___________________________________________________________________________|
*/

#include "stl.h"
#include "math_op.h"
#include "dsputil.h"
#include <stdlib.h>
#include <stdio.h>
/*___________________________________________________________________________
 |                                                                           |
 |   Function Name : Isqrt_lc                                                |
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
static const Word32 L_table_isqrt[48] =
{
     2147418112L,  2083389440L,  2024669184L,  1970667520L,
     1920794624L,  1874460672L,  1831403520L,  1791098880L,
     1753415680L,  1717960704L,  1684602880L,  1653145600L,
     1623326720L,  1595080704L,  1568276480L,  1542782976L,
     1518469120L,  1495334912L,  1473183744L,  1451950080L,
     1431633920L,  1412169728L,  1393491968L,  1375469568L,
     1358168064L,  1341521920L,  1325465600L,  1309933568L,
     1294991360L,  1280507904L,  1266548736L,  1252982784L,
     1239875584L,  1227161600L,  1214775296L,  1202847744L,
     1191182336L,  1179910144L,  1168965632L,  1158283264L,
     1147863040L,  1137770496L,  1127940096L,  1118306304L,
     1108934656L,  1099825152L,  1090912256L,  1082261504L
};
/* table of table_isqrt[i] - table_isqrt[i+1] */
static const Word16 table_isqrt_diff[48] =
{
      977,   896,   824,   761,   707,   657,   615,   575,
      541,   509,   480,   455,   431,   409,   389,   371,
      353,   338,   324,   310,   297,   285,   275,   264,
      254,   245,   237,   228,   221,   213,   207,   200,
      194,   189,   182,   178,   172,   167,   163,   159,
      154,   150,   147,   143,   139,   136,   132,   130
};
static const Word16 shift[] = {9,10};
Word32 Isqrt_lc(
     Word32 frac,  /* (i)   Q31: normalized value (1.0 < frac <= 0.5) */
     Word16 * exp  /* (i/o)    : exponent (value = frac x 2^exponent) */
)
{
    Word16 i, a;
    Word32 L_tmp;

    IF (frac <= (Word32) 0)
    {
        *exp = 0;                          move16();

        return 0x7fffffff;
    }

    /* If exponant odd -> shift right by 10 (otherwise 9) */
    L_tmp = L_shr(frac, shift[s_and(*exp, 1)]);

    /* 1) -16384 to shift left and change sign                 */
    /* 2) 32768 to Add 1 to Exponent like it was divided by 2  */
    /* 3) We let the mac_r add another 0.5 because it imitates */
    /*    the behavior of shr on negative number that should   */
    /*    not be rounded towards negative infinity.            */
    /* It replaces:                                            */
    /*    *exp = negate(shr(sub(*exp, 1), 1));   move16();     */
    *exp = mac_r(32768, *exp, -16384);     move16();

    a = extract_l(L_tmp);                           /* Extract b10-b24 */
    a = lshr(a, 1);

    i = mac_r(L_tmp, -16*2-1, 16384);               /* Extract b25-b31 minus 16 */
    
    L_tmp = L_msu(L_table_isqrt[i], table_isqrt_diff[i], a);/* table[i] << 16 - diff*a*2 */

    return L_tmp;
}

/*___________________________________________________________________________
 |                                                                           |
 |   Function Name : Pow2()                                                  |
 |                                                                           |
 |     L_x = pow(2.0, exponant.fraction)         (exponant = interger part)  |
 |         = pow(2.0, 0.fraction) << exponant                                |
 |---------------------------------------------------------------------------|
 |  Algorithm:                                                               |
 |                                                                           |
 |   The function Pow2(L_x) is approximated by a table and linear            |
 |   interpolation.                                                          |
 |                                                                           |
 |   1- i = bit10-b15 of fraction,   0 <= i <= 31                            |
 |   2- a = bit0-b9   of fraction                                            |
 |   3- L_x = table[i]<<16 - (table[i] - table[i+1]) * a * 2                 |
 |   4- L_x = L_x >> (30-exponant)     (with rounding)                       |
 |___________________________________________________________________________|
*/
static const Word16 table_pow2[32] =
{
  16384, 16743, 17109, 17484, 17867, 18258, 18658, 19066, 
  19484, 19911, 20347, 20792, 21247, 21713, 22188, 22674, 
  23170, 23678, 24196, 24726, 25268, 25821, 26386, 26964, 
  27554, 28158, 28774, 29405, 30048, 30706, 31379, 32066
};

static const Word32 L_deposit_h_table_pow2[32] =
{
    1073741824, 1097269248, 1121255424, 1145831424, 1170931712, 1196556288, 1222770688, 1249509376, 
    1276903424, 1304887296, 1333460992, 1362624512, 1392443392, 1422983168, 1454112768, 1485963264, 
    1518469120, 1551761408, 1585709056, 1620443136, 1655963648, 1692205056, 1729232896, 1767112704, 
    1805778944, 1845362688, 1885732864, 1927086080, 1969225728, 2012348416, 2056454144, 2101477376 
};

/* table of table_pow2[i+1] - table_pow2[i] */
static const Word16 table_pow2_diff_x32[32] =
{
    11488, 11712, 12000, 12256, 12512, 12800, 13056, 13376, 
    13664, 13952, 14240, 14560, 14912, 15200, 15552, 15872, 
    16256, 16576, 16960, 17344, 17696, 18080, 18496, 18880, 
    19328, 19712, 20192, 20576, 21056, 21536, 21984, 22432
};

Word32 Pow2(                              /* (o) Q0  : result       (range: 0<=val<=0x7fffffff) */
    Word16 exponant,                      /* (i) Q0  : Integer part.      (range: 0<=val<=30)   */
    Word16 fraction                       /* (i) Q15 : Fractionnal part.  (range: 0.0<=val<1.0) */
)
{
    Word16 exp, i, a;
    Word32 L_x;

    i = mac_r(-32768, fraction, 32);         /* Extract b10-b16 of fraction */
    a = s_and(fraction, 0x3ff);              /* Extract  b0-b9  of fraction */

    L_x = L_deposit_h_table_pow2[i];           /* table[i] << 16   */

    L_x = L_mac(L_x, table_pow2_diff_x32[i], a);/* L_x -= diff*a*2  */

    exp = sub(30, exponant);

    L_x = L_shr_r(L_x, exp);

    return L_x;
}

static const Word32 L_inv_table[32] = { /* in Q31 */
    2147483647L, 2082408386L, 2021161080L, 1963413621L,
    1908874354L, 1857283155L, 1808407283L, 1762037865L,
    1717986918L, 1676084798L, 1636178018L, 1598127366L,
    1561806289L, 1527099483L, 1493901668L, 1462116526L,
    1431655765L, 1402438301L, 1374389535L, 1347440720L,
    1321528399L, 1296593901L, 1272582903L, 1249445032L,
    1227133513L, 1205604855L, 1184818564L, 1164736894L,
    1145324612L, 1126548799L, 1108378657L, 1090785345L
};    

static const Word16 inv_table_diff[32] = { /* in Q20 */
    31775, 29906, 28197, 26631, 25191, 23865, 22641, 21509,
    20460, 19486, 18579, 17735, 16947, 16210, 15520, 14873,
    14266, 13696, 13159, 12653, 12175, 11724, 11298, 10894,
    10512, 10150,  9806,  9479,  9168,  8872,  8590,  8322
};    

/*---------------------------------------------------------------------------*
 * L_Frac_sqrtQ31
 *
 * Calculate square root from fractional values (Q31 -> Q31)
 * Uses 32 bit internal representation for precision
 *---------------------------------------------------------------------------*/
Word32 L_Frac_sqrtQ31(    /* o  : Square root if input */
    const Word32 x        /* i  : Input                */
)
{
    Word32 log2_work;
    Word16 log2_int, log2_frac;

    test();
    if (x > 0)
    {
        log2_frac = Log2_norm_lc(norm_l_L_shl(&log2_int, x));

        log2_work = L_msu((31+30)*65536L/2, 16384, log2_int);
        log2_work = L_mac0(log2_work, log2_frac, 1);

        log2_frac = L_Extract_lc(log2_work, &log2_int);

        return Pow2(log2_int, log2_frac);
    }
    return 0;
}

/* Square root function : returns sqrt(Num/2) */
/**********************************************/
Word16 L_sqrt(Word32 Num)
{
    Word16    i;
    
    Word16    Rez = (Word16) 0;
    Word16    Exp = (Word16) 0x4000;

    Word32    L_temp;

    Word16 tmp;
    L_temp = L_sub(Num, 536870912);
    if (L_temp >= 0L)
        Rez = add(Rez, Exp);
    Exp = shr(Exp, (Word16) 1);
    FOR(i = 1; i < 14; i++)
    {
        tmp = add(Rez, Exp);
        L_temp = L_msu(Num, tmp, tmp);

        if (L_temp >= 0L)
            Rez = add(Rez, Exp);
        Exp = shr(Exp, (Word16) 1);
    }
    return Rez;
}

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
 |   6- fraction = tablog[i]<<16 - (tablog[i] - tablog[i+1]) * a * 2         |
 |___________________________________________________________________________|
*/

/*-----------------------------------------------------*
 | Table for routine Log2().                           |
 -----------------------------------------------------*/
static const Word32 L_deposit_h_tablog[33] = {
           0,   95354880,  187826176,  277610496,  364904448,  449773568,  532414464,  612892672, 
   691339264,  767819776,  842465280,  915341312,  986578944, 1056243712, 1124335616, 1190920192, 
  1256128512, 1320026112, 1382612992, 1443954688, 1504116736, 1563164672, 1621032960, 1677918208, 
  1733754880, 1788542976, 1842413568, 1895432192, 1947467776, 1998651392, 2049048576, 2098659328,
  2147418112 
};
static Word16 tablog_i_i1[32]={
  -1455, -1411, -1370, -1332, -1295, -1261, -1228, -1197, 
  -1167, -1139, -1112, -1087, -1063, -1039, -1016,  -995,
   -975,  -955,  -936,  -918,  -901,  -883,  -868,  -852,
   -836,  -822,  -809,  -794,  -781,  -769,  -757,  -744
};

static Word16 tablog[33] = {
      0,  1455,  2866,  4236,  5568,  6863,  8124,  9352, 
  10549, 11716, 12855, 13967, 15054, 16117, 17156, 18172, 
  19167, 20142, 21097, 22033, 22951, 23852, 24735, 25603, 
  26455, 27291, 28113, 28922, 29716, 30497, 31266, 32023,
  32767
};
void Log2(Word32 L_x,           /* (i) Q0 : input value                                 */
          Word16 * exponent,    /* (o) Q0 : Integer part of Log2.   (range: 0<=val<=30) */
          Word16 * fraction     /* (o) Q15: Fractional  part of Log2. (range: 0<=val<1) */
    )
{
  Word32    L_y;

  Word16    exp, i, a, tmp;

  if (L_x <= (Word32) 0)
  {
    *exponent = 0;
    *fraction = 0;
    return;
  }

  L_x = norm_l_L_shl(&exp, L_x);

  *exponent = sub(30, exp);

  L_x = L_shr(L_x, 9);
  i = extract_h(L_x);           /* Extract b25-b31 */
  L_x = L_shr(L_x, 1);
  a = extract_l(L_x);           /* Extract b10-b24 of fraction */
  a = s_and( a, (Word16) 0x7fff);

  i = sub(i, 32);

  L_y = L_deposit_h_tablog[i]; /* tablog[i] << 16        */
  tmp = tablog_i_i1[i];  /* tablog[i] - tablog[i+1] */

  L_y = L_msu(L_y, tmp, a);     /* L_y -= tmp*a*2        */

  *fraction = extract_h(L_y);

  return;
}
