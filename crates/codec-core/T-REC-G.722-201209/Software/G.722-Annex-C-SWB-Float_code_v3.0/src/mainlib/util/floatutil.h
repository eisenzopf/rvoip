/* ITU G.722 3rd Edition (2012-09) */

/*--------------------------------------------------------------------------
 ITU-T G.722 Annex C (ex G.722-SWB-Float) Source Code
 Software Release 1.01 (2012-07)
 (C) 2012 France Telecom, Huawei Technologies, VoiceAge Corp., NTT.
--------------------------------------------------------------------------*/

#ifndef FLOATUTIL_H
#define FLOATUTIL_H

#ifdef AVOID_CONFLICT
#define round(a) round32(a)     /* To avoid conflict of round */
#endif

typedef float  Float;
typedef short  Short;

#define Q09F(x)  ((Float)x/512.0f)
#define Q12F(x)  ((Float)x/4096.0f)
#define Q15F(x)  ((Float)x/32768.0f)
#define Q22F(x)  ((Float)x/4194304.0f)
#define FQ12(x)  (roundFto16((Float)x*4096.0f))
#define FQ15(x)  (roundFto16((Float)x*32768.0f))

#define Fabs(x)  ((x)<0?-(x):(x))

void   zeroFloat( int n, Float *x );
void   zeroF( int n, Float *x );
void   zeroS( int n, Short *x );
void   movF( int n, Float *x, Float *y );
void   movSS( int n, Short *x, Short *y );
void   movFF( int n, Float *x, Float *y );
void   movSF( int n, Short *x, Float *y );
void   movSFQ( int n, Short *x, Float *y, int q);
void   movDpfFQ( int n, Short *hi, Short *lo, Float *y, int q);
void   movFS( int n, Float *x, Short *y );
void   movFSQ( int n, Float *x, Short *y, int q );
void   movFDpfQ( int n, Float *x, Short *hi, Short *lo, int q );

Float  Sqrt( Float input );

Short roundFto16( Float x );
long roundFto32( Float x );
void roundFtoDpf( Float x, Short *hi, Short *lo );

Float  Pow( Float x, Float y );

void   SftFto16Array( int n, Float *xin, Short *xout, Short nExp );
Short ExpFto16Array( int n, Float *xin );
Short CnvFto16Array( int n, Float *xin, Short *xout );

int Fnorme16(Float f);
int Fnorme32(Float f);
Float f_max( Float var1, Float var2);
Float f_min( Float var1, Float var2);
Float abs_f (Float var1); /* same as Fabs() */

void movW32F(int n, long *x, Float *y);
void movFW32(int n, Float *x, long *y);
void movW32FQ(int n, long *x, Float *y, int q);
void movFW32Q(int n, Float *x, long *y, int q);

Float sum_vect_E(      /* o:   return calculated vector energy           */
    const Float *vec,  /* i:   input vector                              */
    const Short lvec  /* i:   length of input vector                    */
);

Float  Ceil ( Float input );
Float  Floor ( Float input );

Float  Log10 (Float x);
Float  Log (Float x);
Float  Cos (Float x);

void abs_array_f( Float *a , Float *b , int length );
Float mac0_Array_f( int size , Float *fx , Float *fy );
Float mac_Array_f( int size , Float *fx , Float *fy );
void movF_ext( int size , Float *fx , int m , Float *sy , int l );
void movF_bwd( Float  n , Float  *fx , Float  *fy );
Float  f_Log2 (Float x);
#endif
