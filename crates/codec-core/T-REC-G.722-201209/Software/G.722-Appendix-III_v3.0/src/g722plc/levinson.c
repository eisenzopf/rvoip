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
#if (DMEM)
#include "memutil.h"
#endif
#include "utility.h"

/*___________________________________________________________________________
 |                                                                           |
 |      LEVINSON-DURBIN algorithm in double precision                        |
 |      ~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~                        |
 |---------------------------------------------------------------------------|
 |                                                                           |
 | Algorithm                                                                 |
 |                                                                           |
 |       R[i]    autocorrelations.                                           |
 |       A[i]    filter coefficients.                                        |
 |       K       reflection coefficients.                                    |
 |       Alpha   prediction gain.                                            |
 |                                                                           |
 |       Initialization:                                                     |
 |               A[0] = 1                                                    |
 |               K    = -R[1]/R[0]                                           |
 |               A[1] = K                                                    |
 |               Alpha = R[0] * (1-K**2]                                     |
 |                                                                           |
 |       Do for  i = 2 to m                                                  |
 |                                                                           |
 |            S =  SUM ( R[j]*A[i-j] ,j=1,i-1 ) +  R[i]                      |
 |                                                                           |
 |            K = -S / Alpha                                                 |
 |                                                                           |
 |            An[j] = A[j] + K*A[i-j]   for j=1 to i-1                       |
 |                                      where   An[i] = new A[i]             |
 |            An[i]=K                                                        |
 |                                                                           |
 |            Alpha=Alpha * (1-K**2)                                         |
 |                                                                           |
 |       END                                                                 |
 |                                                                           |
 | Remarks on the dynamics of the calculations.                              |
 |                                                                           |
 |       The numbers used are in double precision in the following format :  |
 |       A = AH <<16 + AL<<1.  AH and AL are 16 bit signed integers.         |
 |       Since the LSB's also contain a sign bit, this format does not       |
 |       correspond to standard 32 bit integers.  We use this format since   |
 |       it allows fast execution of multiplications and divisions.          |
 |                                                                           |
 |       "DPF" will refer to this special format in the following text.      |
 |       See oper_32b.c                                                      |
 |                                                                           |
 |       The R[i] were normalized in routine AUTO (hence, R[i] < 1.0).       |
 |       The K[i] and Alpha are theoretically < 1.0.                         |
 |       The A[i], for a sampling frequency of 8 kHz, are in practice        |
 |       always inferior to 16.0.                                            |
 |                                                                           |
 |       These characteristics allow straigthforward fixed-point             |
 |       implementation.  We choose to represent the parameters as           |
 |       follows :                                                           |
 |                                                                           |
 |               R[i]    Q31   +- .99..                                      |
 |               K[i]    Q31   +- .99..                                      |
 |               Alpha   Normalized -> mantissa in Q31 plus exponent         |
 |               A[i]    Q27   +- 15.999..                                   |
 |                                                                           |
 |       The additions are performed in 32 bit.  For the summation used      |
 |       to calculate the K[i], we multiply numbers in Q31 by numbers        |
 |       in Q27, with the result of the multiplications in Q27,              |
 |       resulting in a dynamic of +- 16.  This is sufficient to avoid       |
 |       overflow, since the final result of the summation is                |
 |       necessarily < 1.0 as both the K[i] and Alpha are                    |
 |       theoretically < 1.0.                                                |
 |___________________________________________________________________________|
*/


/* Last A(z) for case of unstable filter */

#define	M	16

void Levinson(
  Word32 Rhl[],     /* (i)       : Rh[M+1] Vector of autocorrelations (msb) */
  Word16 A[],       /* (o) Q12   : A[M]    LPC coefficients                 */
  Word16 old_A[],   /* (i/o) Q12 : old_A[M+1] old LPC coefficients          */
  Word16 m	        /* (i)       : LPC order				    */
)
{
 Word16 i, j;
 Word16 hi, lo;
 Word16 Kh, Kl;                /* reflection coefficient; hi and lo           */
 Word16 alp_h, alp_l, alp_exp; /* Prediction gain; hi lo and exponent         */
 Word32 t0, t1, t2;            /* temporary variable                          */
 Word16  *pAh, *pAl;
 Word16 unstable;

#if (DMEM)
 Word16 *Ah, *Al;      /* LPC coef. in double prec.                   */
 Word16 *Anh, *Anl;    /* LPC coef.for next iteration in double prec. */
#else
 Word16 Ah[M+1], Al[M+1];      /* LPC coef. in double prec.                   */
 Word16 Anh[M+1], Anl[M+1];    /* LPC coef.for next iteration in double prec. */
#endif

/* K = A[1] = -R[1] / R[0] */

#if (DMEM)
  /* memory allocation */
  Ah  = allocWord16(0, m);
  Al  = allocWord16(0, m);
  Anh = allocWord16(0, m);
  Anl = allocWord16(0, m);
#endif

  t1  = Rhl[1]; 				        /* R[1] in Q31      */
  unstable = 0;
#if WMOPS
  move32();move16();
#endif

  t2  = L_abs(t1);                      /* abs R[1]         */
  L_Extract(Rhl[0], &alp_h, &alp_l);	 /* borrow alp for Rhl[0] */
  t0  = Div_32(t2, alp_h, alp_l);       /* R[1]/R[0] in Q31 */
  if (t1 > 0) t0= L_negate(t0);         /* -R[1]/R[0]       */
  L_Extract(t0, &Kh, &Kl);              /* K in DPF         */
  t0 = L_shr(t0,4);                     /* A[1] in Q27      */
  L_Extract(t0, &Ah[1], &Al[1]);        /* A[1] in DPF      */

/*  Alpha = R[0] * (1-K**2) */

  t0 = Mpy_32(Kh ,Kl, Kh, Kl);          /* K*K      in Q31 */
  t0 = L_abs(t0);                       /* Some case <0 !! */
  t0 = L_sub( (Word32)0x7fffffffL, t0 );/* 1 - K*K  in Q31 */
  L_Extract(t0, &hi, &lo);              /* DPF format      */
  t0 = Mpy_32(alp_h , alp_l, hi, lo);   /* Alpha in Q31    */

/* Normalize Alpha */

  alp_exp = norm_l(t0);
  t0 = L_shl(t0, alp_exp);
  L_Extract(t0, &alp_h, &alp_l);         /* DPF format    */

/*--------------------------------------*
 * ITERATIONS  I=2 to m                 *
 *--------------------------------------*/

  FOR (i= 2; i<=m; i++)
  {

    /* t0 = SUM ( R[j]*A[i-j] ,j=1,i-1 ) +  R[i] */

    L_Extract(Rhl[1], &hi, &lo);
     pAh = Ah+i-1;
     pAl = Al+i-1;
     t0 = Mpy_32(hi, lo, *pAh--, *pAl--);
     FOR(j=2; j<i; j++) {
	      L_Extract(Rhl[j], &hi, &lo);	
         t0 = L_add(t0, Mpy_32(hi, lo, *pAh--, *pAl--));
	}

    t0 = L_shl(t0,4);                  /* result in Q27 -> convert to Q31 */
                                       /* No overflow possible            */
    t1 = Rhl[i];
    t0 = L_add(t0, t1);                /* add R[i] in Q31                 */

    /* K = -t0 / Alpha */

    t1 = L_abs(t0);
    t2 = Div_32(t1, alp_h, alp_l);     /* abs(t0)/Alpha                   */
    if(t0 > 0) t2= L_negate(t2);       /* K =-t0/Alpha                    */
    t2 = L_shl(t2, alp_exp);           /* denormalize; compare to Alpha   */
    L_Extract(t2, &Kh, &Kl);           /* K in DPF                        */

    /* Test for unstable filter.*/
    if (sub(abs_s(Kh), 32750) > 0)
         unstable = 1;

    /*------------------------------------------*
     *  Compute new LPC coeff. -> An[i]         *
     *  An[j]= A[j] + K*A[i-j]     , j=1 to i-1 *
     *  An[i]= K                                *
     *------------------------------------------*/

    pAh = Ah+i-1;
    pAl = Al+i-1;
    FOR(j=1; j<i; j++)
    {
      t0 = Mpy_32(Kh, Kl, *pAh--, *pAl--);
      t0 = L_add(t0, L_Comp(Ah[j], Al[j]));
      L_Extract(t0, &Anh[j], &Anl[j]);
    }
    t2 = L_shr(t2, 4);                  /* t2 = K in Q31 ->convert to Q27  */
    L_Extract(t2, &Anh[i], &Anl[i]);    /* An[i] in Q27                    */

    /*  Alpha = Alpha * (1-K**2) */

    t0 = Mpy_32(Kh ,Kl, Kh, Kl);          /* K*K      in Q31 */
    t0 = L_abs(t0);                       /* Some case <0 !! */
    t0 = L_sub( (Word32)0x7fffffffL, t0 );/* 1 - K*K  in Q31 */
    L_Extract(t0, &hi, &lo);              /* DPF format      */
    t0 = Mpy_32(alp_h , alp_l, hi, lo);   /* Alpha in Q31    */

    /* Normalize Alpha */

    j = norm_l(t0);
    t0 = L_shl(t0, j);
    L_Extract(t0, &alp_h, &alp_l);         /* DPF format    */
    alp_exp = add(alp_exp, j);             /* Add normalization to alp_exp */

    /* A[j] = An[j] */

    FOR(j=1; j<=i; j++)
    {
      Ah[j] =Anh[j];
      Al[j] =Anl[j];
#if WMOPS
      move16();move16();
#endif
    }
  }
  /* Test for unstable filter. If unstable keep old A(z) */
  IF (unstable)
  {
      FOR(j=0; j<=m; j++)
      {
        A[j] = old_A[j];
#ifdef WMOPS
        move16();
#endif
      }

#if (DMEM)
      /* memory deallocation */
      deallocWord16(Ah,  0, m);
      deallocWord16(Al,  0, m);
      deallocWord16(Anh, 0, m);
      deallocWord16(Anl, 0, m);
#endif

      return;
  }

  /* Truncate A[i] in Q27 to Q12 with rounding */

  A[0] = 4096;
#if WMOPS
  move16();
#endif
  FOR(i=1; i<=m; i++)
  {
    t0   = L_Comp(Ah[i], Al[i]);
    old_A[i] = A[i] = round(L_shl(t0, 1));
#if WMOPS
    move16();move16();
#endif
  }

#if (DMEM)
  /* memory deallocation */
  deallocWord16(Ah,  0, m);
  deallocWord16(Al,  0, m);
  deallocWord16(Anh, 0, m);
  deallocWord16(Anl, 0, m);
#endif

  return;
}

