/* ITU G.722 3rd Edition (2012-09) */

/* ITU-T G.722 Appendix III                                                      */
/* Version:       1.0                                                            */
/* Revision Date: Nov.02, 2006                                                   */

/*
  ITU-T G.722 Appendix III ANSI-C Source Code
  Copyright (c)  Broadcom Corporation 2006.
*/


#include "typedef.h"
#include "g722plc.h"
#include "stl.h"
#if (DMEM)
#include "memutil.h"
#endif
#include "utility.h"


/*-----------------------------------------------------------------------------
 * Function: Autocorr()
 *
 * Description: Calculates autocorrelation.
 *
 * Inputs:  x[]      - Input signal
 *          window[] - LPC Analysis window
 *          l_window - window length
 *          m        - LPC order
 *
 * Outputs: r[]      - Autocorrelations
 *---------------------------------------------------------------------------*/
void Autocorr(
Word32	r[],
Word16	x[],
Word16	window[],
Word16 	l_window,
Word16 	m)
{
  Word16 i, j, norm;
#if (DMEM)
  Word16 *y;
#else
  Word16 y[WINSZ];
#endif
  Word32 sum;
  Word16 lw;

  extern Flag Overflow;

#if (DMEM)
  /* memory allocation */
  y = allocWord16(0, l_window-1);
#endif

  /* Windowing of signal */

  FOR(i=0; i<l_window; i++)
  {
    y[i] = mult_r(x[i], window[i]);
#ifdef WMOPS
    move16();
#endif
  }

  /* Compute r[0] and test for overflow */

  DO {
    Overflow = 0;
    sum = 1;                   /* Avoid case of all zeros */
#if WMOPS
    move16(); move16();
#endif
    FOR(i=0; i<l_window; i++)
      sum = L_mac0(sum, y[i], y[i]);

    /* If overflow divide y[] by 4 */

    IF (Overflow) {

      FOR(i=0; i<l_window; i++) 
      {
         y[i] = shr(y[i], 2);
#ifdef WMOPS
         move16();
#endif
      }
    }

  } WHILE (Overflow);

  /* Normalization of r[0] */

  norm = norm_l(sum);
  r[0]  = L_shl(sum, norm);
#ifdef WMOPS
  move16();
#endif
  /* r[1] to r[m] */

  FOR (i = 1; i <= m; i++)
  {
    sum=L_mult0(y[0],y[i]);
    lw = sub(l_window, i);
    FOR(j=1; j<lw; j++) sum = L_mac0(sum, y[j], y[j+i]);

    r[i] = L_shl(sum, norm);
#ifdef WMOPS
    move16();
#endif
  }

#if (DMEM)
  /* dememory allocation */
  deallocWord16(y, 0, l_window-1);
#endif

}


/*-----------------------------------------------------------------------------
 * Function: Spectral_Smoothing()
 *
 * Description: Performs spectral smoothing on the autocorrelation coefficients.
 *
 * Inputs:  m       - LPC order
 *          r[]     - Autocorrelations
 *          lag_h[] - SST coefficients  (msb)
 *          lag_l[] - SST coefficients  (lsb)
 *
 * Outputs: r[]     - Autocorrelations
 *---------------------------------------------------------------------------*/
void Spectral_Smoothing(
  Word16 m,
  Word32 r[],
  Word16 lag_h[],
  Word16 lag_l[]
)
{
  Word16 	i;
  Word16	hi, lo;

#if WMOPS
  move16(); /* for loading of lag_h[i-1] pointer */
  move16(); /* for loading of lag_l[i-1] pointer */
#endif
  FOR(i=1; i<=m; i++)
  {
  	 L_Extract(r[i], &hi, &lo);
    r[i] = Mpy_32(hi, lo, lag_h[i-1], lag_l[i-1]);
#ifdef WMOPS
    move16();
#endif
  }
}
