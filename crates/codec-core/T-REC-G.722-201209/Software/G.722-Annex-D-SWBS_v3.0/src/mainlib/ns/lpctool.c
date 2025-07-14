/* ITU G.722 3rd Edition (2012-09) */

/*-----------------------------------------------------------------------------------
 ITU-T G.722-SWBS / G.722 Annex D - Reference C code for fixed-point implementation          
 Version 1.0
 Copyright (c) 2012,
 Huawei Technologies
-----------------------------------------------------------------------------------*/

/*--------------------------------------------------------------------------
 ITU-T Annex B (ex G.722-SWB) Source Code
 Software Release 1.00 (2010-09)
 (C) 2010 France Telecom, Huawei Technologies, NTT, VoiceAge Corp.
--------------------------------------------------------------------------*/

/*
*------------------------------------------------------------------------
*  File: lpctool.c
*  Function: Linear prediction tools
*------------------------------------------------------------------------
*/

#include "pcmswb_common.h"
#include "oper_32b.h"
#include "lpctool.h"

#include "dsputil.h"

/*****************************/
#ifdef DYN_RAM_CNT
#include "dyn_ram_cnt.h"
#endif
/*****************************/

/*-------------------------------------------------------------------------*
* Function Levinson                                                        *
*--------------------------------------------------------------------------*/

#define MAXORD  6

void Levinson(
              Word16 R_h[],     /* (i)     : Rh[M+1] Vector of autocorrelations (msb) */
              Word16 R_l[],     /* (i)     : Rl[M+1] Vector of autocorrelations (lsb) */
              Word16 rc[],      /* (o) Q15 : rc[M]   Reflection coefficients.         */
              Word16 * stable,  /* (o)     : Stability flag                           */
              Word16 ord,       /* (i)     : LPC order                                */
              Word16 * a        /* (o) Q12 : LPC coefficients                         */
              )
{
  Word32  t0, t1, t2;                     /* temporary variable */
  Word16 *A_h;/* LPC coef. in double prec.*/
  Word16 *A_l;/* LPC coef. in double prec.*/
  Word16 *An_h; /* LPC coef. for next iteration in double prec.  */
  Word16 *An_l; /* LPC coef. for next iteration in double prec.  */
  Word16  i, j;
  Word16  hi, lo;
  Word16  K_h, K_l;                       /* reflexion coefficient; hi and lo */
  Word16  alp_h, alp_l, alp_exp;          /* Prediction gain; hi lo and exponent */

  i = add(ord,1);
  A_h = (Word16 *)calloc(i, sizeof(Word16));
  A_l = (Word16 *)calloc(i, sizeof(Word16));
  An_h = (Word16 *)calloc(i, sizeof(Word16));
  An_l = (Word16 *)calloc(i, sizeof(Word16));

#ifdef DYN_RAM_CNT
  {
    UWord32 ssize = 0;
    ssize += (UWord32) ((4) * SIZE_Ptr);
    ssize += (UWord32) ((9) * SIZE_Word16);
    ssize += (UWord32) ((4*i) * SIZE_Word16); /*calloc-s*/
    ssize += (UWord32) ((3) * SIZE_Word32);
    DYN_RAM_PUSH(ssize, "dummy");
  }
#endif

  *stable = 0; move16();

  /* K = A[1] = -R[1] / R[0] */

  t1 = L_Comp(R_h[1], R_l[1]);            /* R[1]             */
  t2 = L_abs(t1);                         /* abs R[1]         */
  t0 = Div_32(t2, R_h[0], R_l[0]);        /* R[1]/R[0] in Q31 */

  if (t1 > 0)
  {
    t0 = L_negate(t0);                    /* -R[1]/R[0] */
  }

  K_l = L_Extract_lc(t0, &K_h);              /* K in DPF */

  rc[0] = K_h; move16();
  t0 = L_shr(t0, 4);
  L_Extract(t0, &A_h[1], &A_l[1]);        /* A[1] in DPF */

  /* Alpha = R[0] * (1-K**2) */

  t0 = Mpy_32(K_h, K_l, K_h, K_l);        /* K*K */
  t0 = L_abs(t0);                         /* Some case <0 !! */
  t0 = L_sub((Word32) 0x7fffffffL, t0);   /* 1 - K*K */

  lo = L_Extract_lc(t0, &hi);

  t0 = Mpy_32(R_h[0], R_l[0], hi, lo);    /* Alpha in DPF format */

  /* Normalize Alpha */
  t0 = norm_l_L_shl(&alp_exp, t0);

  alp_l = L_Extract_lc(t0, &alp_h);          /* DPF format */

  /*-------------------------------------- */
  /* ITERATIONS  I=2 to lpc_order          */
  /*-------------------------------------- */

  FOR (i = 2; i <= ord; i++)
  {
    /* t0 = SUM ( R[j]*A[i-j] ,j=1,i-1 ) + R[i] */

    t0 = Mpy_32(R_h[1], R_l[1], A_h[i - 1], A_l[i - 1]);   
    FOR (j = 2; j < i; j++)
    {
      t0 = Mac_Mpy_32(t0, R_h[j], R_l[j], A_h[i - j], A_l[i - j]);
    }
    t0 = L_shl(t0, 4);

    t0 = L_msu(t0, R_h[i], -32768);
    t0 = L_mac (t0, R_l[i], 1);

    /* K = -t0 / Alpha */

    t1 = L_abs(t0);
    t2 = Div_32(t1, alp_h, alp_l);        /* abs(t0)/Alpha */

    if (t0 > 0)
    {
      t2 = L_negate(t2);                  /* K =-t0/Alpha */
    }
    t2 = L_shl(t2, alp_exp);              /* denormalize; compare to Alpha */

    K_l = L_Extract_lc(t2, &K_h);            /* K in DPF */

    rc[i - 1] = K_h; move16();   

    /* Test for unstable filter. If unstable keep old A(z) */

    IF (sub(abs_s(K_h), 32750) > 0)
    {
      *stable = 1; move16();
      free(A_h);
      free(A_l);
      free(An_h);
      free(An_l);

      /*****************************/
#ifdef DYN_RAM_CNT
      DYN_RAM_POP();
#endif
      /*****************************/
      return;
    }

    /*------------------------------------------ */
    /*    Compute new LPC coeff. -> An[i]        */
    /*    An[j]= A[j] + K*A[i-j]   , j=1 to i-1  */
    /*    An[i]= K                               */
    /*------------------------------------------ */

    FOR (j = 1; j < i; j++)
    {
      t0 = Mpy_32(K_h, K_l, A_h[i - j], A_l[i - j]);   

      t0 = L_msu(t0, A_h[j], -32768);
      t0 = L_mac (t0, A_l[j], 1);

      L_Extract(t0, &An_h[j], &An_l[j]);
    }
    t2 = L_shr(t2, 4);
    L_Extract(t2, &An_h[i], &An_l[i]);

    /* Alpha = Alpha * (1-K**2) */

    t0 = Mpy_32(K_h, K_l, K_h, K_l);      /* K*K */
    t0 = L_abs(t0);                       /* Some case <0 !! */
    t0 = L_sub((Word32) 0x7fffffffL, t0); /* 1 - K*K */

    lo = L_Extract_lc(t0, &hi);              /* DPF format */

    t0 = Mpy_32(alp_h, alp_l, hi, lo);

    /* Normalize Alpha */
    t0 = norm_l_L_shl(&j, t0);

    alp_l = L_Extract_lc(t0, &alp_h);        /* DPF format */

    alp_exp = add(alp_exp, j);            /* Add normalization to alp_exp */

    /* A[j] = An[j] */

    FOR (j = 1; j <= i; j++)
    {
      A_h[j] = An_h[j]; move16();
      A_l[j] = An_l[j]; move16();
    }
  }

  a[0] = 4096; move16();

  FOR (i = 1; i <= ord; i++)
  {
    t0 = L_deposit_h (A_h[i]);
    t0 = L_shl(t0, 1);
    a[i] = mac_r (t0, A_l[i], 2); move16();
  }
  free(A_h);
  free(A_l);
  free(An_h);
  free(An_l);

  /*****************************/
#ifdef DYN_RAM_CNT
  DYN_RAM_POP();
#endif
  /*****************************/
  return;
}

/*----------------------------------------------------------*
* Function Lag_window()                                    *
*                                                          *
* r[i] *= lag_wind[i]                                      *
*                                                          *
*    r[i] and lag_wind[i] are in special double precision. *
*    See "oper_32b.c" for the format                       *
*                                                          *
*----------------------------------------------------------*/

void Lag_window(
                Word16 * R_h,
                Word16 * R_l,
                const Word16 * W_h,
                const Word16 * W_l,
                Word16 ord
                )
{
  Word32  x;
  Word16  i;
#ifdef DYN_RAM_CNT
  {
    UWord32 ssize = 0;
    ssize += (UWord32) ((0) * SIZE_Ptr);
    ssize += (UWord32) ((1) * SIZE_Word16);
    ssize += (UWord32) ((1) * SIZE_Word32);
    DYN_RAM_PUSH(ssize, "dummy");
  }
#endif

  FOR (i = 1; i <= ord; i++)
  {
    x = Mpy_32(R_h[i], R_l[i], W_h[i - 1], W_l[i - 1]);
    L_Extract(x, &R_h[i], &R_l[i]);
  }
  /*****************************/
#ifdef DYN_RAM_CNT
  DYN_RAM_POP();
#endif
  /*****************************/
  return;
}

/*------------------------------------------------------------------------*
*                         WEIGHT_A.C                                     *
*------------------------------------------------------------------------*
*   Weighting of LPC coefficients                                        *
*   ap[i]  =  a[i] * (gamma ** i)                                        *
*                                                                        *
*------------------------------------------------------------------------*/

void Weight_a(
              Word16 a[],        /* (i) Q*  : a[m+1]  LPC coefficients             */
              Word16 ap[],       /* (o) Q*  : Spectral expanded LPC coefficients   */
              Word16 gamma,      /* (i) Q15 : Spectral expansion factor.           */
              Word16 m           /* (i)     : LPC order.                           */
              )
{
  Word16 i, fac;
#ifdef DYN_RAM_CNT
  {
    UWord32 ssize = 0;
    ssize += (UWord32) ((0) * SIZE_Ptr);
    ssize += (UWord32) ((2) * SIZE_Word16);
    ssize += (UWord32) ((0) * SIZE_Word32);
    DYN_RAM_PUSH(ssize, "dummy");
  }
#endif

  ap[0] = a[0]; move16();
  fac = gamma;  move16();
  FOR (i = 1; i < m; i++)
  {
    ap[i] = mult_r (a[i], fac); move16();
    fac = mult_r (fac, gamma);
  }
  ap[m] = mult_r (a[m], fac); move16();

  /*****************************/
#ifdef DYN_RAM_CNT
  DYN_RAM_POP();
#endif
  /*****************************/
  return;
}
