/* ITU G.722 3rd Edition (2012-09) */

/*-----------------------------------------------------------------------------------
 ITU-T G.722-SWBS / G.722 Annex D - Reference C code for fixed-point implementation          
 Version 1.0
 Copyright (c) 2012,
 Huawei Technologies, France Telecom
-----------------------------------------------------------------------------------*/

/*--------------------------------------------------------------------------
 ITU-T Annex B (ex G.722-SWB) Source Code
 Software Release 1.00 (2010-09)
 (C) 2010 France Telecom, Huawei Technologies, NTT, VoiceAge Corp.
--------------------------------------------------------------------------*/

#include "dsputil.h"
#include "pcmswb.h"

#ifdef LAYER_STEREO
Word16    Exp32Array(
                     Word16     n,     /* (i): Array size   */
                     Word32  *sx    /* (i): Data array   */
                     )
{
  Word16     k;
  Word16     exp;
  Word32  L_Max;
  Word32  L_Abs;

  L_Max = L_abs( sx[0] );
  FOR ( k = 1; k < n; k++ )
  {
    L_Abs = L_abs( sx[k] );
    L_Max = L_max( L_Max, L_Abs );
  }
  exp = norm_l( L_Max );

  if(L_Max == 0)
  {
      exp = 31; move16();
  }
  return exp;
}
#endif

void L_mac_shr(Word16 len, Word32 *L_temp, Word16 b, Word16 *spit)
{
  Word16 j, temp;

  temp = shr(*spit, b);
  *L_temp = L_mac(0, temp, temp);
  spit++;
  FOR (j = 1; j < len; j++)
  {
    temp = shr(*spit, b);
    *L_temp = L_mac(*L_temp, temp, temp);
    spit++;
  }
}

/*----------------------------------------------------------------
Function:
Fills zeros in an array.
Return value
None
----------------------------------------------------------------*/
void zero16(
            Word16  n,     /* I : */
            Word16  *sx    /* O : */
            )
{
  Word16 k;

  FOR ( k = 0; k < n; k++ )
  {
    sx[k] = 0; move16();
  }
}

void zero16_8(
            Word16  *sx    /* O : */
            )
{
  sx[0] = 0; move16();
  sx[1] = 0; move16();
  sx[2] = 0; move16();
  sx[3] = 0; move16();
  sx[4] = 0; move16();
  sx[5] = 0; move16();
  sx[6] = 0; move16();
  sx[7] = 0; move16();
}

/*----------------------------------------------------------------
Function:
Copies array data. 
Permits also to shify en array toward smaller indexes
For example shift of the array s of n elements by 1
mov16(n-1, &s[1], &s[0]) or mov16(n-1, s+1, s)
s[0] = s[1]; s[1] = s[2]; s[2] = s[3]; ...s[n-2] = s[n-1]; 
generalisation: shift by k elements: mov16(n-k, s+k, s)
Return value
None
----------------------------------------------------------------*/
void mov16(
           Word16  n,     /* I : */
           Word16  *sx,   /* I : */
           Word16  *sy    /* O : */
           )
{
  Word16 k;

  FOR ( k = 0; k < n; k++ )
  {
    sy[k] = sx[k]; move16();
  }
}

void mov16_8(
           Word16  *sx,   /* I : */
           Word16  *sy    /* O : */
           )
{
  sy[0] = sx[0]; move16();
  sy[1] = sx[1]; move16();
  sy[2] = sx[2]; move16();
  sy[3] = sx[3]; move16();
  sy[4] = sx[4]; move16();
  sy[5] = sx[5]; move16();
  sy[6] = sx[6]; move16();
  sy[7] = sx[7]; move16();
}

/*----------------------------------------------------------------
Function:
Copies array data by decrementing insexes. 
Permits also to shify en array toward higher indexes
For example shift of the array s of n elements by 1
mov16bwd(n-1, &s[n-2], &s[n-1]) or mov16bwd(n-1, s+n-2, s+n-1)
s[n-1] = s[n-2]; s[n-2] = s[n-3]; s[n-3] = s[n-4]; ...s[1] = s[0]; 
generalisation: shift by k elements: mov16(n-k, s+n-k-1, s+n-1)
Return value
None
----------------------------------------------------------------*/
void mov16_bwd(
               Word16  n,     /* I : */
               Word16  *sx,   /* I : */
               Word16  *sy    /* O : */
               )
{
  Word16 k;

  FOR ( k = 0; k < n; k++ )
  {
    *sy-- = *sx--; move16();
  }
}

/*----------------------------------------------------------------
Function:
Finds number of shifts to normalize a 16-bit array variable.
Return value
Number of shifts
----------------------------------------------------------------*/
Word16    Exp16Array(
                     Word16     n,     /* (i): Array size   */
                     Word16  *sx    /* (i): Data array   */
                     )
{
  Word16     k;
  Word16     exp;
  Word16  sMax;
  Word16  sAbs;

  sMax = abs_s( sx[0] ); move16();

  FOR ( k = 1; k < n; k++ )
  {
    sAbs = abs_s( sx[k] );
    sMax = s_max( sMax, sAbs );
  }

  exp = norm_s( sMax );
  return exp;
}

/*----------------------------------------------------------------
Function:
Bounds a 16-bit value between x_min and x_max.
Return value
the bounded value
----------------------------------------------------------------*/
Word16    bound(
                Word16     x,     /* (i): input value   */
                Word16  x_min,    /* (i): lower limit   */
                Word16  x_max     /* (i): higher limit   */
                )
{
  x = s_max(x, x_min);
  x = s_min(x, x_max);
  return x;
}

/*----------------------------------------------------------------
Function:
computes L_mac0 of two 16-bit array variables.
Return value the found max value
L_mac0 results
----------------------------------------------------------------*/
Word32    L_mac0_Array(
                       Word16     n,      /* (i): Array size    */
                       Word16  *sx,       /* (i): Data array 1  */
                       Word16  *sy        /* (i): Data array 2  */
                       )
{
  Word32     x;
  Word16     k;

  x = L_mult0(sx[0], sy[0]); 

  FOR ( k = 1; k < n; k++ )
  {
    x = L_mac0(x, sx[k], sy[k]);
  }

  return x;
}

/*----------------------------------------------------------------
Function:
computes L_mac of two 16-bit array variables.
Return value the found max value
L_mac results
----------------------------------------------------------------*/
Word32    L_mac_Array(
                      Word16     n,      /* (i): Array size    */
                      Word16  *sx,       /* (i): Data array 1  */
                      Word16  *sy        /* (i): Data array 2  */
                      )
{
  Word32     x;
  Word16     k;

  x = L_mult(sx[0], sy[0]); 

  FOR ( k = 1; k < n; k++ )
  {
    x = L_mac(x, sx[k], sy[k]);
  }

  return x;
}

Word32    L_mac_Array8(
                      Word16    a,       /* (i): initial value */
                      Word16  *sx,       /* (i): Data array 1  */
                      Word16  *sy        /* (i): Data array 2  */
                      )
{
  Word32     x;
  x = L_mac(a, sx[0], sy[0]); 
  x = L_mac(x, sx[1], sy[1]);
  x = L_mac(x, sx[2], sy[2]);
  x = L_mac(x, sx[3], sy[3]);
  x = L_mac(x, sx[4], sy[4]);
  x = L_mac(x, sx[5], sy[5]);
  x = L_mac(x, sx[6], sy[6]);
  x = L_mac(x, sx[7], sy[7]);

  return x;
}

/*---------------------------------------------------------------------*
* procedure   Sum_vect_E:                                     
*             ~~~~~~~~~~                                    
*  Find vector energy
*---------------------------------------------------------------------*/
/*static*/ Word32 Sum_vect_E8( /* OUT:   return calculated vector energy */
                             const Word16 *vec      /* IN:   input vector                     */
                             )
{
  Word32  L_sum;

  L_sum = L_mult0(vec[0], vec[0]);
  L_sum = L_mac0(L_sum, vec[1], vec[1]);
  L_sum = L_mac0(L_sum, vec[2], vec[2]);
  L_sum = L_mac0(L_sum, vec[3], vec[3]);
  L_sum = L_mac0(L_sum, vec[4], vec[4]);
  L_sum = L_mac0(L_sum, vec[5], vec[5]);
  L_sum = L_mac0(L_sum, vec[6], vec[6]);
  L_sum = L_mac0(L_sum, vec[7], vec[7]);
  return L_sum;
}

Word16  MaxArray(
                 Word16     n,      /* (i): Array size   */
                 Word16  *sx,       /* (i): Data array   */
                 Word16  *ind       /* (o): index of max   */
                 )
{
  Word16     k;
  Word16  sMax;

  sMax = sx[0]; move16();
  *ind = 0; move16();

  FOR ( k = 1; k < n; k++ )
  {
    if ( sub( sMax, sx[k] ) < 0 )
    {
      *ind = k; move16();
      sMax = sx[k]; move16();
    }
  }

  return sMax;
}

Word32 L_add_Array(
                   Word16     n,      /* (i): Array size   */
                   Word16  *sx       /* (i): Data array   */
                   )
{
  Word32 L_temp;  
  Word16 i;
  L_temp = L_deposit_l(sx[0]);
  FOR (i = 1; i < n; i++)
  {
    L_temp = L_mac0(L_temp, sx[i], 1);  
  }
  return L_temp;
}

void const16(
             Word16  n,     /* I : length */
             Word16 con,    /* I : const */
             Word16  *sx    /* O : */
             )
{
  Word16 k;

  FOR ( k = 0; k < n; k++ )
  {
    sx[k] = con; move16();
  }
}

void mov16_ext(
               Word16  n,     /* I : */
               Word16  *sx,   /* I : */
               Word16  m,     /* I : */
               Word16  *sy,   /* O : */
               Word16  l      /* I : */
               )
{
  Word16 k;

  FOR ( k = 0; k < n; k++ )
  {
    *sy = *sx; move16();
    sx += m;
    sy += l;
  }
}

Word16 extract_h_L_shl(Word32 t32, Word16 b)
{
  return extract_h(L_shl(t32, b));
}
Word16 extract_h_L_shr_sub(Word32 L_tmp, Word16 a, Word16 b)
{
  return extract_h(L_shr(L_tmp, sub(a,b)));
}
Word16 round_fx_L_shl(Word32 a, Word16 b)
{
  return round_fx(L_shl(a, b));
}
Word16 round_fx_L_shl_L_mult(Word16 a, Word16 b, Word16 c)
{
  return round_fx_L_shl(L_mult(a, b), c);
}

void abs_array(Word16 *a, Word16 *b, Word16 L) {
  Word16 i;
  FOR(i=0; i<L; i++)
  {
    b[i] = abs_s(a[i]); move16();
  }
  return;
}

void array_oper(
                Word16     n,     /* I : */
                Word16     b,     /* I : */
                Word16     *sx,   /* I : */
                Word16     *sy,   /* O : */
                Word16     (*ptf)(Word16, Word16)
                )
{
  Word16 k;

  FOR ( k = 0; k < n; k++ )
  {
    sy[k] = ptf(sx[k], b); move16();
  }
}

void array_oper8(
                Word16     b,     /* I : */
                Word16     *sx,   /* I : */
                Word16     *sy,   /* O : */
                Word16     (*ptf)(Word16, Word16)
                )
{
  sy[0] = ptf(sx[0], b); move16();
  sy[1] = ptf(sx[1], b); move16();
  sy[2] = ptf(sx[2], b); move16();
  sy[3] = ptf(sx[3], b); move16();
  sy[4] = ptf(sx[4], b); move16();
  sy[5] = ptf(sx[5], b); move16();
  sy[6] = ptf(sx[6], b); move16();
  sy[7] = ptf(sx[7], b); move16();
}

void array_oper_ext(
                    Word16  n,     /* I : */
                    Word16  b,     /* I : */
                    Word16  *sx,   /* I : */
                    Word16  m,     /* I : */
                    Word16  *sy,   /* O : */
                    Word16  l ,    /* I : */
                    Word16     (*ptf)(Word16, Word16)
                    )
{
  Word16 k;

  FOR ( k = 0; k < n; k++ )
  {
    *sy = ptf(*sx, b); move16();
    sx += m;
    sy += l;
  }
}

Word32 norm_l_L_shl(Word16 *exp_den, Word32 L_en)
{
  *exp_den = norm_l(L_en);
  return L_shl(L_en, *exp_den);
}
Word16 round_fx_L_shr_L_mult(Word16 a, Word16 b, Word16 c)
{
  return round_fx(L_shr(L_mult(a, b) ,c));
}

Word16 extract_l_L_shr(Word32 a, Word16 b)
{
  return extract_l(L_shr(a, b));
}

Word32 L_abs_L_deposit_l(Word16 a)
{
  return L_abs(L_deposit_l(a));
}
void FOR_L_mult_L_shr_L_add(Word16 a, Word16 *spMDCT_wb, Word16 b, Word32 *L_temp1, Word32 *L_temp)
{
  Word16 i;

  *L_temp = L_mult(*spMDCT_wb, *spMDCT_wb);            /* Q(2*norm_MDCT_fix+1) */
  *L_temp1 = L_shr(*L_temp, b);
  spMDCT_wb++;
  FOR(i=1; i<a; i++)
  {
    *L_temp = L_mult(*spMDCT_wb, *spMDCT_wb);            /* Q(2*norm_MDCT_fix+1) */
    *L_temp = L_shr(*L_temp, b);
    *L_temp1 = L_add(*L_temp1, *L_temp);
    spMDCT_wb++;
  }
}

Word32 Mac_Mpy_32 (Word32 L_32, Word16 hi1, Word16 lo1, Word16 hi2, Word16 lo2)
{
  L_32 = L_mac (L_32, hi1, hi2);
  L_32 = L_mac (L_32, mult (hi1, lo2), 1);
  L_32 = L_mac (L_32, mult (lo1, hi2), 1);

  return (L_32);
}
