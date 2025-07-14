/* ITU G.722 3rd Edition (2012-09) */

/*--------------------------------------------------------------------------
 ITU-T G.722 Annex C (ex G.722-SWB-Float) Source Code
 Software Release 1.01 (2012-07)
 (C) 2012 France Telecom, Huawei Technologies, VoiceAge Corp., NTT.
--------------------------------------------------------------------------*/

/*
*------------------------------------------------------------------------
*  File: lpctool.c
*  Function: Linear prediction tools
*------------------------------------------------------------------------
*/
#include <math.h>

#include "pcmswb_common.h"
#include "lpctool.h"
/*-------------------------------------------------------------------------*
* Function Levinson                                                        *
*--------------------------------------------------------------------------*/

#define MAXORD  6


void fl_Levinson(
              Float R[],     /* (i)     : R[M+1] Vector of autocorrelations  */
              Float rc[],      /* (o)   : rc[M]   Reflection coefficients.         */
              Short *stable,  /* (o)    : Stability flag                           */
              Short ord,       /* (i)   : LPC order                                */
              Float * a        /* (o)   : LPC coefficients                         */
              )
{
  Float  err, s, at ;                     /* temporary variable */
  int   i, j, l;

  *stable = 0; 

  /* K = A[1] = -R[1] / R[0] */
  rc[0] = (-R[1]) / R[0];
  a[0] = (Float) 1.0;
  a[1] = rc[0];
  err = R[0] + R[1] * rc[0];
  
  /*-------------------------------------- */
  /* ITERATIONS  I=2 to lpc_order          */
  /*-------------------------------------- */
  for (i = 2; i <= ord; i++) {
	  s = (Float) 0.0;
	  for (j = 0; j < i; j++) {
		  s += R[i - j] * a[j];
	  }
	  rc[i - 1] = (-s) / (err);
	  /* Test for unstable filter. If unstable keep old A(z) */
	  if(fabs(rc[i-1])> 0.99) {
		  *stable = 1; 
		  return;
	  }

	  for (j = 1; j <= (i / 2); j++) {
		  l = i - j;
		  at = a[j] + rc[i - 1] * a[l];
		  a[l] += rc[i - 1] * a[j];
		  a[j] = at;
	  }
	  a[i] = rc[i - 1];
	  err += rc[i - 1] * s;
	  if (err <= (Float) 0.0) {
		  err = (Float) 0.001;
	  }

  }
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

void fl_Lag_window(
                Float * R,
                const Float * W,
                Short ord
                )
{
  int i;

  for (i = 1; i <= ord; i++)
  {
	  R[i] *= W[i - 1];
  }
  return;
}


/*------------------------------------------------------------------------*
*                         WEIGHT_A.C                                     *
*------------------------------------------------------------------------*
*   Weighting of LPC coefficients                                        *
*   ap[i]  =  a[i] * (gamma ** i)                                        *
*                                                                        *
*------------------------------------------------------------------------*/

void fl_Weight_a(
              Float a[],        /* (i)  : a[m+1]  LPC coefficients             */
              Float ap[],       /* (o)  : Spectral expanded LPC coefficients   */
              Float gamma,      /* (i)  : Spectral expansion factor.           */
              Short m           /* (i)  : LPC order.                           */
              )
{
  int i;
  Float fac;

  ap[0] = a[0]; 
  fac = gamma;  
  for (i = 1; i < m; i++)
  {
	  ap[i] = fac * a[i];
	  fac *= gamma;
  }
  ap[m] = a[m]* fac; 

  return;
}
