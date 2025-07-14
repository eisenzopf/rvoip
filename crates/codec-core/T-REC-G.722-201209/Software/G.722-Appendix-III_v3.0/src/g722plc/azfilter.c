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


/*-----------------------------------------------------------------------------
 * Function: azfilterQ0_Q1()
 *
 * Description: Performs all-zero filtering with Q0 in and Q1 out.
 *
 * Inputs:  a[]   - prediction coefficients, Q12
 *          m     - LPC order
 *          x[]   - input signal
 *          lg    - size of filtering
 *
 * Outputs: y[]   - output signal
 *---------------------------------------------------------------------------*/
void azfilterQ0_Q1(
  Word16 a[],
  Word16 m,
  Word16 x[],
  Word16 y[],
  Word16 lg
)
{
  Word16 i;
  Word32 s;

   FOR (i = 0; i < lg; i++) 
   {
      s = L_mult0(x[i], a[0]); /* Q12 */
      s = L_mac0(s, a[1], x[i-1]);
      s = L_mac0(s, a[2], x[i-2]);
      s = L_mac0(s, a[3], x[i-3]);
      s = L_mac0(s, a[4], x[i-4]);
      s = L_mac0(s, a[5], x[i-5]);
      s = L_mac0(s, a[6], x[i-6]);
      s = L_mac0(s, a[7], x[i-7]);
      s = L_mac0(s, a[8], x[i-8]);

      y[i] = round(L_shl(s, 5)); /* Q1 */
#ifdef WMOPS
      move16();
#endif
   }
}
