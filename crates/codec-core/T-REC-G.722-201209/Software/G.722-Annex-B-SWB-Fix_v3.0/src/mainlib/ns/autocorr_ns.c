/* ITU G.722 3rd Edition (2012-09) */

/*--------------------------------------------------------------------------
 ITU-T Annex B (ex G.722-SWB) Source Code
 Software Release 1.00 (2010-09)
 (C) 2010 France Telecom, Huawei Technologies, NTT, VoiceAge Corp.
--------------------------------------------------------------------------*/

/*
*------------------------------------------------------------------------
*  File: autocorr_ns.c
*  Function: Compute autocorrelations of signal for noise shaping
*------------------------------------------------------------------------
*/

#include "pcmswb_common.h"
#include "oper_32b.h"
#include "ns.h"
#include "lpctool.h"

/*****************************/
#ifdef DYN_RAM_CNT
#include "dyn_ram_cnt.h"
#endif
/*****************************/

Word16 AutocorrNS(  /*  Return: R0 Normalization shift       */
                  Word16  x[],      /* (i)    : Input signal (80 samples)    */
                  Word16  r_h[],    /* (o) DPF: Autocorrelations (high)      */
                  Word16  r_l[]     /* (o) DPF: Autocorrelations (low)       */
)
{
  Word16  i, j, norm, alpha, y[L_WINDOW];
  Word32  L_sum;
  Word16  sshift;
  Word16 tmp;

  /* Approximate R(1)/R(0) (tilt or harmonicity) with a zero-crossing measure */
  Word16 zcross = L_WINDOW-1;
  move16();

#ifdef DYN_RAM_CNT
  {
    UWord32 ssize = 0;
    ssize += (UWord32) ((0) * SIZE_Ptr);
    ssize += (UWord32) ((7 + L_WINDOW) * SIZE_Word16);
    ssize += (UWord32) ((1) * SIZE_Word32);
    DYN_RAM_PUSH(ssize, "dummy");
  }
#endif

  FOR (i = 1; i < L_WINDOW; ++i) {
    if (s_xor(x[i-1],x[i]) < 0) {
      zcross = sub(zcross,1);
    }
  }
  zcross = add(12543,shl(zcross,8)); /* set the factor between .38 and 1.0 */

  /* Pre-emphesis and windowing */
  FOR (i = 1; i < L_WINDOW; i++) {
    move16();
    /* Emphasize harmonic signals more than noise-like signals */
    y[i] = mult_r(NS_window[i],sub(x[i],mult_r(zcross,x[i-1])));
  }

  /* Low level fixed noise shaping (when rms <= 100) */
  alpha = 100;  move16();

  L_sum = L_mult(alpha, 100);

  FOR (i = 1; i < L_WINDOW; i++) {
    L_sum = L_mac(L_sum, y[i], y[i]);
  }

  sshift = 0;  move16();

  IF ( L_sub(L_sum, MAX_32) == 0 )    /* Overflow */
  {
    sshift = 2;  move16();
    alpha = 25;  move16();

    L_sum = L_mult(alpha, 25);
    FOR (i = 1; i < L_WINDOW; i++) {
      y[i] = shr (y[i], 2); move16();
      L_sum = L_mac(L_sum, y[i], y[i]);
    }
  }

  alpha = mult(alpha, 31130);         /* alpha *= 0.95 */

  L_sum = norm_l_L_shl(&norm, L_sum);
  L_Extract(L_sum, &r_h[0], &r_l[0]); /* Put in DPF format */

  /* Compute r[1] to r[m] */
  FOR (i = 1; i <= ORD_M; i++)
  {
    /* low level fix noise shaping */

    L_sum = L_mult(alpha, shr(100,sshift));
    alpha = mult(alpha, 31130);       /* alpha *= 0.95 */

    tmp = sub(L_WINDOW,i);
    FOR (j = 1; j < tmp; j++) {
      L_sum = L_mac(L_sum, y[j], y[j+i]);
    }
    L_sum = L_shl(L_sum, norm);
    L_Extract(L_sum, &r_h[i], &r_l[i]);
  }

  /* Lag windowing */
  Lag_window(r_h, r_l, NS_lag_h, NS_lag_l, ORD_M);

  /*****************************/
#ifdef DYN_RAM_CNT
  DYN_RAM_POP();
#endif
  /*****************************/
  return norm;
}
