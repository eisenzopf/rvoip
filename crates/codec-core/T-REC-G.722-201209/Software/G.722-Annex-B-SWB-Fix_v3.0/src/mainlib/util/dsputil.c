/* ITU G.722 3rd Edition (2012-09) */

/*--------------------------------------------------------------------------
 ITU-T Annex B (ex G.722-SWB) Source Code
 Software Release 1.00 (2010-09)
 (C) 2010 France Telecom, Huawei Technologies, NTT, VoiceAge Corp.
--------------------------------------------------------------------------*/

#include "dsputil.h"

void L_mac_shr(Word16 len, Word32 *L_temp, Word16 b, Word16 *spit)
{
  Word16 j, temp;
  *L_temp = 0; move32();
  FOR (j = 0; j < len; j++)
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

  sMax = 0; move16();

  FOR ( k = 0; k < n; k++ )
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

/*---------------------------------------------------------------------*
* procedure   Sum_vect_E:                                     
*             ~~~~~~~~~~                                    
*  Find vector energy
*---------------------------------------------------------------------*/

/*static*/ Word32 Sum_vect_E( /* OUT:   return calculated vector energy */
                             const Word16 *vec,      /* IN:   input vector                     */
                             const Word16 lvec       /* IN:   length of input vector           */
                             )
{
  Word16  j;
  Word32  L_sum;

  L_sum = L_mult0(*vec, *vec);
  FOR (j = 1 ; j < lvec; j++)
  {
    L_sum = L_mac0(L_sum, vec[j], vec[j]);
  }
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
  *L_temp1 = 0; move32();
  FOR(i=0; i<a; i++)
  {
    *L_temp = L_mult(*spMDCT_wb, *spMDCT_wb);            /* Q(2*norm_MDCT_fix+1) */
    *L_temp = L_shr(*L_temp, b);
    *L_temp1 = L_add(*L_temp1, *L_temp);
    spMDCT_wb++;
  }
}
