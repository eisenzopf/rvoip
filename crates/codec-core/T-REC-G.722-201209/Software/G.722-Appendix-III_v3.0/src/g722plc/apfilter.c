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
#include "utility.h"
#if (DMEM)
#include "memutil.h"
#endif

#if (!DMEM)
#define BUFFERSIZE  (8+320)
#endif


/*-----------------------------------------------------------------------------
 * Function: apfilterQ0_Q0()
 *
 * Description: Performs all-pole filtering with Q0 in and out.
 *
 * Inputs:  a[]   - prediction coefficients, Q12
 *          m     - LPC order
 *          x[]   - input signal
 *          lg    - size of filtering
 *          mem[] - filter memory
 *
 * Outputs: y[]   - output signal
 *          mem[] - filter memory
 *---------------------------------------------------------------------------*/
void apfilterQ0_Q0(
  Word16 a[],
  Word16 m,
  Word16 x[],
  Word16 y[],
  Word16 lg,
  Word16 mem[]
  )
{
  Word16 i, j;
  Word32 s;
#if (DMEM)
  Word16 *tmp;
#else
  Word16 tmp[BUFFERSIZE];
#endif
  Word16 *yy;

#if (DMEM)
  /* memory allocation */
  tmp = allocWord16(0, m+lg-1);
#endif

  /* Copy mem[] to yy[] */

  W16copy(tmp, mem, m);
  yy = &tmp[m];

  /* Do the filtering. */

  FOR (i = 0; i < lg; i++) {
    s = L_mult0(x[i], a[0]); /* Q12 */
    FOR (j = 1; j <= m; j++) s = L_msu0(s, a[j], yy[-j]); /* Q12 */
    *yy++ = round(L_shl(s, 4)); /* Q0 */
#ifdef WMOPS
    move16();
#endif
  }
  W16copy(y, &tmp[m], lg);

#if (DMEM)
  /* memory deallocation */
  deallocWord16(tmp, 0, m+lg-1);
#endif

}


/*-----------------------------------------------------------------------------
 * Function: apfilterQ1_Q0()
 *
 * Description: Performs all-pole filtering with Q1 in and Q0 out.
 *
 * Inputs:  a[]   - prediction coefficients, Q12
 *          m     - LPC order
 *          x[]   - input signal
 *          lg    - size of filtering
 *          mem[] - filter memory
 *
 * Outputs: y[]   - output signal
 *          mem[] - filter memory
 *---------------------------------------------------------------------------*/
void apfilterQ1_Q0(
  Word16 a[],
  Word16 m,
  Word16 x[],
  Word16 y[],
  Word16 lg,
  Word16 mem[]
)
{
   Word16 i;
   Word32 s;

   /* Copy mem[] to y[] */
   W16copy(y-m, mem, m);

   /* Do the filtering. */
   FOR (i = 0; i < lg; i++) 
   {
      s = L_mult0(x[i], a[0]); /* Q13 */
      s = L_msu(s, a[1], y[-1]); /* Q13 */
      s = L_msu(s, a[2], y[-2]); /* Q13 */
      s = L_msu(s, a[3], y[-3]); /* Q13 */
      s = L_msu(s, a[4], y[-4]); /* Q13 */
      s = L_msu(s, a[5], y[-5]); /* Q13 */
      s = L_msu(s, a[6], y[-6]); /* Q13 */
      s = L_msu(s, a[7], y[-7]); /* Q13 */
      s = L_msu(s, a[8], y[-8]); /* Q13 */
      *y++ = round(L_shl(s, 3)); /* Q0 */
#ifdef WMOPS
      move16();
#endif
   }
}

