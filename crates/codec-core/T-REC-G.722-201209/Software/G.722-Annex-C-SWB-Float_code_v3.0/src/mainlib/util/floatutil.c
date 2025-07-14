/* ITU G.722 3rd Edition (2012-09) */

/*--------------------------------------------------------------------------
 ITU-T G.722 Annex C (ex G.722-SWB-Float) Source Code
 Software Release 1.01 (2012-07)
 (C) 2012 France Telecom, Huawei Technologies, VoiceAge Corp., NTT.
--------------------------------------------------------------------------*/

#define  AVOID_CONFLICT
#include "floatutil.h"
#include <stdio.h>
#include <stdlib.h>
#include <math.h>

/*__________________________________________________________________________
|                                                                          |
| zero()                                                                   |
|__________________________________________________________________________|
*/
void zeroFloat(
           int  n, /* I : */
           Float *x /* O : */
           )
{
  int k;
  
  for(k = 0; k < n; k++ ) {
    x[k] = (Float)0.0;
  }
}

void zeroF(
           int  n, /* I : */
           Float *x /* O : */
           )
{
  int k;
  
  for(k = 0; k < n; k++ ) {
    x[k] = (Float)0.0;
  }
}

void zeroS(
           int  n, /* I : */
           Short *x /* O : */
           )
{
  int k;

  for(k = 0; k < n; k++) {
    x[k] = 0;
  }
}

/*__________________________________________________________________________
|                                                                          |
| mov()                                                                    |
|__________________________________________________________________________|
*/
void movF(
          int  n, /* I : */
          Float *x, /* I : */
          Float *y /* O : */
          )
{
  int k;

  for(k = 0; k < n; k++) {
    y[k] = x[k];
  }
}

void movSS(
           int  n, /* I : */
           Short *x, /* I : */
           Short *y /* O : */
           )
{
  int k;

  for(k = 0; k < n; k++) {
    y[k] = x[k];
  }
}

void movSF(
           int  n, /* I : */
           Short *x, /* I : */
           Float *y /* O : */
           )
{
  int k;

  for(k = 0; k < n; k++) {
    y[k] = (Float)x[k];
  }
}

void movSFQ(
            int  n, /* I : */
            Short *x, /* I : */
            Float *y, /* O : */
            int  q   /* I : Q value (example 15 for Q15)*/
            )
{
  int k;
  Float fact;

  fact = (Float)(pow(2, q));

  for(k = 0; k < n; k++) {
    y[k] = (Float)(x[k] / fact);
  }
}

void movDpfFQ(
              int  n, /* I : */
              Short *hi,/* I : */
              Short *lo,/* I : */
              Float *y, /* O : */
              int  q   /* I : Q value (example 15 for Q15)*/
              )
{
  int k;
  long  s32;
  Float fact;

  fact = (Float)(pow(2, q));
  for(k = 0; k < n; k++) {
    s32 = (hi[k]<<16) + (lo[k]<<1);
    y[k] = (Float)(s32 / fact);
  }
}

void movFS(
           int  n, /* I : */
           Float *x, /* I : */
           Short *y /* O : */
           )
{
  int k;

  for(k = 0; k < n; k++) {
    y[k] = roundFto16(x[k]);
  }
}

void movFSQ(
            int  n, /* I : */
            Float *x, /* I : */
            Short *y, /* O : */
            int  q   /* I : Q value (example 15 for Q15)*/
            )
{
  int k;
  Float fact;

  fact = (Float)(pow(2, q));
  for(k = 0; k < n; k++) {
    y[k] = roundFto16(x[k] * fact);

  }
}

void movFDpfQ(
              int  n, /* I : */
              Float *x, /* I : */
              Short *hi,/* O : */
              Short *lo,/* O : */
              int  q   /* I : Q value (example 15 for Q15)*/
              )
{
  int k;
  Float fact;

  fact = (Float)(pow(2, q));
  for(k = 0; k < n; k++) {
    roundFtoDpf(x[k] * fact, hi+k, lo+k);
  }
}

/*__________________________________________________________________________
|                                                                          |
| sqrt()                                                                    |
|__________________________________________________________________________|
*/
Float Sqrt(Float input)
{
  return (Float)sqrt((double)input);
}

/*__________________________________________________________________________
|                                                                          |
| roundFto16()                                                             |
|__________________________________________________________________________|
*/
Short roundFto16(Float x)
{
  Short  out;

  if (x >= 32767.0) {
    out = 32767;
  }
  else if (x <= -32768.0) {
    out = -32768;
  }
  else if (x >= 0.0) {
    out = (Short)(x + 0.5);
  }
  else {
    out = (Short)(x - 0.5);
  }
  return out;
}

long roundFto32(Float x)
{
  long  out;

  if (x >= 2147483647.0) {
    out = 2147483647L;
  }
  else if (x <= -2147483648.0) {
    out = -2147483647L-1L;
  }
  else if (x >= 0.0) {
    out = (long)(x + 0.5);
  }
  else {
    out = (long)(x - 0.5);
  }
  return out;
}

void roundFtoDpf(Float x, Short *hi, Short *lo)
{
  long  s32;

  s32 = roundFto32(x);
  *hi = (Short)(s32 >> 16);
  *lo = (Short)((s32 - (*hi<<16)) >> 1) ; 
}
/*__________________________________________________________________________
|                                                                          |
| pow()                                                                    |
|__________________________________________________________________________|
*/
Float Pow(Float x, Float y)
{
  return (Float)pow((double)x, (double)y);
}

/*__________________________________________________________________________
|                                                                          |
| SftFto16Array()                                                                    |
|__________________________________________________________________________|
*/
void SftFto16Array(
                   int  n,
                   Float *xin,
                   Short  *xout,
                   Short  nExp
                   ) {
                     int   i;
                     Float a;

                     a = 1.0;
                     if (nExp >= 0) {
                       for(i = 0; i < nExp; i++) {
                         a *= 2.0;
                       }
                     }
                     else {
                       for(i = 0; i < (-nExp); i++) {
                         a /= 2.0;
                       }
                     }

                     for (i = 0; i < n; i++) {
                       if (xin[i] < 0.0) {
                         xout[i] = -(Short)(Fabs(xin[i])*a  +0.5);
                       }
                       else {
                         xout[i] = (Short)(xin[i]*a + 0.5);
                       }
                     }
}

Short ExpFto16Array(
                    int  n,
                    Float *xin
                    ) {
                      int   i;
                      int   count;
                      Float xmax, xabs;
                      Float a;

                      xmax = 0.0;
                      for (i = 0; i < n; i++) {
                        xabs = Fabs(xin[i]);
                        if( xmax < xabs )
                          xmax = xabs;
                      }

                      if (xmax < 1./32768.) {
                        return 0;
                      }

                      if (xmax > 0xFFFFFFFFUL) {
                        fprintf( stderr, "Floating utility error.\n" );
                        exit(1);
                      }

                      count = 0;
                      a = 1.0;
                      for (i = 0; i < 31; i++) {
                        if ((unsigned long)(xmax*a+0.5) < 0x4000) {
                          a *= 2.0;
                          count++;
                        }
                        else if ((unsigned long)(xmax*a+0.5) > 0x7FFF) {
                          a /= 2.0;
                          count--;
                        }
                        else {
                          break;
                        }
                      }

                      if ((unsigned long)(xmax*a+0.5) > 0x7FFF) {
                        fprintf( stderr, "Floating utility error.\n" );
                        exit(1);
                      }

                      return count;
}


Short CnvFto16Array(
                    int  n,
                    Float *xin,
                    Short  *xout
                    ) {
                      Short nExp;

                      nExp = ExpFto16Array( n, xin );
                      SftFto16Array( n, xin, xout, nExp );

                      return nExp;
}

/* simulate the 'norm16' function of stl2005*/
int Fnorme16(Float f)
{
  int q = 0;
  if (f == (Float)0)
  {
    return(q);
  }
  else
  {
    if (f < (Float)0)
    {
      f = -f;
    }
    while(f < 16384) /*2^14*/
    {
      f *= 2;
      q++;
    }
    return(q);
  }
}

/* simulate the 'norm32' function of stl2005*/
int Fnorme32(Float f)
{
  int q = 0;
  if (f == (Float)0)
  {
    return(q);
  }
  else
  {
    while(f < 1073741824L) /*2^30*/
    {
      f *= 2;
      q++;
    }
  }
  return(q);
}

/* Max function for floating point data */
Float f_max( Float var1, Float var2) 
{
  Float var_out;

  if( var1 >= var2)
    var_out = var1;
  else
    var_out = var2;

  return( var_out);
}

/* Min function for floating point data */
Float f_min(Float var1, Float var2)
{
  Float var_out;

  if( var1 <= var2)
    var_out = var1;
  else
    var_out = var2;

  return( var_out);
}

/* Abs function for floating point data */
Float abs_f (Float var1)
{
  if (var1 < 0)
  {
    var1 = -var1;
  }
  return (var1);
}

/* long to Float data conversion */
void movW32F(
             int     n,  /* I : */
             long  *x, /* I : */
             Float   *y  /* O : */
             )
{
  int k;

  for( k = 0; k < n; k++ )
  {
    y[k] = (Float)x[k];
  }
}

/* Float to long data conversion */
void movFW32(
             int     n,  /* I : */
             Float   *x, /* I : */
             long  *y  /* O : */
             )
{
  int k;

  for( k = 0; k < n; k++ )
  {
    y[k] = roundFto32(x[k]);
  }
}

/* long with Q-value to Float data conversion */
void movW32FQ(
              int     n,  /* I : */
              long  *x, /* I : */
              Float   *y, /* O : */
              int     q   /* I : Q value (example 15 for Q15)*/
              )
{
  int k;
  Float fact;

  fact = (Float)(pow(2, q));
  for( k = 0; k < n; k++ )
  {
    y[k] = (Float)(x[k] / fact);
  }
}

/* Float to long with Q-value data conversion */
void movFW32Q(
              int     n,  /* I : */
              Float   *x, /* I : */
              long  *y, /* O : */
              int     q   /* I : Q value (example 15 for Q15)*/
              )
{
  int k;
  Float fact;

  fact = (Float)(pow(2, q));
  for( k = 0; k < n; k++ )
  {
    y[k] = roundFto32(x[k] * fact);
  }
}

/*---------------------------------------------------------------------*
* procedure   sum_vect_E:                                     
*             ~~~~~~~~~~                                    
*  Find vector energy
*---------------------------------------------------------------------*/

Float sum_vect_E(      /* o:   return calculated vector energy           */
                 const Float *vec,  /* i:   input vector                              */
                 const Short lvec  /* i:   length of input vector                    */
                 )
{
  int j;
  Float suma = 0;

  for (j = 0 ; j < lvec ; j++)
  {
    suma += vec[j] * vec[j];
  }
  return suma;
}


/*__________________________________________________________________________
|                                                                          |
| ceil ()                                                                  |
|__________________________________________________________________________|
*/
Float Ceil ( Float input )
{
  return (Float) ceil ( (double)input );
}

/*__________________________________________________________________________
|                                                                          |
| floor ()                                                                 |
|__________________________________________________________________________|
*/
Float Floor ( Float input )
{
  return (Float) floor ( (double)input );
}

Float Log10(Float x)
{
    return (Float) log10 ((double)x);
}

Float Log(Float x)
{
    return (Float) log ((double)x);
}

Float Cos(Float x)
{
    return (Float) cos ((double)x);
}

Float f_Log2(Float x)
{
	Float ans;
    ans =  (Float)log(x) / (Float)log(2);

	return ans;
}


//
//abs_array
//
void abs_array_f( Float *a , Float *b , int length )
{
	int i;
	for( i=0 ; i<length ; i++ ){
		b[i] = abs_f( a[i] );
	}
	return;
}


//
//L_mac0_array
//
Float mac0_Array_f( int size , Float *fx , Float *fy )
{
	Float x = 0;
	int i;

	for( i=0 ; i<size ; i++ ){
		x = x + ( fx[i] * fy[i] );
	}
	return x;
}


//
//L_mac_array
//
Float mac_Array_f( int size , Float *fx , Float *fy )
{
	Float x = 0;
	int i;

	for( i=0 ; i<size ; i++ ){
		x = x + ( fx[i] * fy[i] );
	}
	return x;
}


//
//mov_16_ext
//
void movF_ext( int size , Float *fx , int m , Float *fy , int l )
{
  int k;

  for( k = 0 ; k<size ; k++ )
  {
    *fy = *fx;
    fx += m;
    fy += l;
  }
}


void movF_bwd(
               Float  n,     /* I : */
               Float  *fx,   /* I : */
               Float  *fy    /* O : */
               )
{
  int k;

  for( k = 0; k < n; k++ )
  {
    *fy-- = *fx--;
  }
}
