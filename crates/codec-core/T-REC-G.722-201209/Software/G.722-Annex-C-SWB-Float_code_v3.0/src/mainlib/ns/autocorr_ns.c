/* ITU G.722 3rd Edition (2012-09) */

/*--------------------------------------------------------------------------
 ITU-T G.722 Annex C (ex G.722-SWB-Float) Source Code
 Software Release 1.01 (2012-07)
 (C) 2012 France Telecom, Huawei Technologies, VoiceAge Corp., NTT.
--------------------------------------------------------------------------*/

/*
*------------------------------------------------------------------------
*  File: autocorr_ns.c
*  Function: Compute autocorrelations of signal for noise shaping
*------------------------------------------------------------------------
*/
#include "floatutil.h"
#include "pcmswb_common.h"
#include "ns.h"
#include "lpctool.h"


Short fl_AutocorrNS(  /*  Return: R0 Normalization shift       */
                  Float  x[],      /* (i)    : Input signal (80 samples)    */
                  Float  r[]    /* (o) : Autocorrelations */
)
{
  int i, j;
  Float  alpha, y[L_WINDOW];
  Float  sum, zcr;
  Short norm;

  /* Approximate R(1)/R(0) (tilt or harmonicity) with a zero-crossing measure */
  Short zcross;
  
  zcross = L_WINDOW-1;
  for (i = 1; i < L_WINDOW; ++i) 
  {
	  if((x[i-1] )< 0.0f) 
	  {
		  if(x[i]>=0.0f) zcross--;
	  }
	  else
	  {
		  if(x[i]<0.0f) zcross--;
	  }

  }
  zcr = (Float)0.38275+(Float)(zcross)*(Float)0.007813; /* set the factor between .38 and 1.0 */

  /* Pre-emphesis and windowing */
  for (i = 1; i < L_WINDOW; i++) {
    /* Emphasize harmonic signals more than noise-like signals */
    y[i] = fl_NS_window[i]* (x[i]-zcr*x[i-1]);
  }

  /* Low level fixed noise shaping (when rms <= 100) */
  
  sum = (Float)10000.0; /* alpha* alpha */
  for (i = 1; i < L_WINDOW; i++) {
    sum += y[i]* y[i];
  }
  r[0] = sum;
  alpha = (Float)1.;  

  /* Compute r[1] to r[m] */
  for (i = 1; i <= ORD_M; i++)
  {
	  /* low level fix noise shaping */
	  alpha *= (Float)0.95;       /* alpha *= 0.95 */
	  sum = alpha * (Float)10000.0;
	  for (j = 1; j < L_WINDOW-i; j++) {
		  sum += y[j] * y[j+i];
	  }
	  r[i] = sum;
  }

  /* Lag windowing */
  fl_Lag_window(r, fl_NS_lag, ORD_M);

  norm = (Short)Fnorme32((Float)2.*r[0]);

  return norm;
}
