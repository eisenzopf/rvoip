/* ITU G.722 3rd Edition (2012-09) */

/*--------------------------------------------------------------------------
 ITU-T G.722 Annex C (ex G.722-SWB-Float) Source Code
 Software Release 1.01 (2012-07)
 (C) 2012 France Telecom, Huawei Technologies, VoiceAge Corp., NTT.
--------------------------------------------------------------------------*/

/*
 *------------------------------------------------------------------------
 *  File: prehpf.c
 *  Function: Pre-processing 1-tap high-pass filtering
 *            Cut-off (-3dB) frequency is approximately 50 Hz,
 *            if the recommended filt_no value is used.
 *------------------------------------------------------------------------
 */

#include "pcmswb_common.h"
#include "prehpf.h"

#define Q_14  16384.0f

typedef struct {
  Float f_memx;
  Float f_memy;
} HPASSMEM;

/* Constructor */
void  *highpass_1tap_iir_const(void)  /* returns pointer to work space */
{
  HPASSMEM *hpmem;

  hpmem = (HPASSMEM *)malloc( sizeof(HPASSMEM) );

  if ( hpmem != NULL )
    highpass_1tap_iir_reset( (void *)hpmem );
  return (void *)hpmem;
}

/* Destructor */
void  highpass_1tap_iir_dest(void *ptr)
{
  HPASSMEM *hpmem = (HPASSMEM *)ptr;	
  if (hpmem != NULL )
  {
    free( hpmem );
  }
}

/* Reset */
void  highpass_1tap_iir_reset(void *ptr)
{
  HPASSMEM *hpmem = (HPASSMEM *)ptr;
  if (hpmem != NULL) {
    hpmem->f_memx = 0.0f;
    hpmem->f_memy = 0.0f;
  }
}

/* Filering */
void  highpass_1tap_iir(
  Short filt_no,  /* (i):   Filter cutoff specification. */
                  /*        Use 5 for 8-kHz input,       */
                  /*            6 for 16-kHz input,      */
                  /*            7 for 32-kHz input       */
  Short n,        /* (i):   Number of samples            */
  Short sigin[],  /* (i):   Input signal                 */
  Float sigout[], /* (o):   Output signal                */
  void  *ptr      /* (i/o): Work space                   */
) 
{
  Float   lAcc;
  int     k;
  Float   sigpre;
  Float   acc;
  HPASSMEM *hpmem = (HPASSMEM *) ptr;

  acc = hpmem->f_memy;
  sigpre = hpmem->f_memx;

  for (k = 0; k < n; k++) {
	/* y[k] = a * y[k-1] + x[k] - x[k-1] */
    lAcc = Floor( acc * Q_14 );
    lAcc = Floor( lAcc / Pow( 2.0f , (Float)filt_no )) / Q_14;
    acc = (Floor(acc * Q_14) - Floor(lAcc * Q_14)) / Q_14;
    acc = (Floor( acc * Q_14 ) + Floor( (Float)*sigin * Q_14 )) / Q_14;
    acc = (Floor( acc * Q_14 ) - Floor( sigpre * Q_14 )) / Q_14;
    sigpre = *sigin++;
    *sigout++ = (Float)roundFto16( acc );
  }
  hpmem->f_memx = sigpre;
  hpmem->f_memy = acc;
}
