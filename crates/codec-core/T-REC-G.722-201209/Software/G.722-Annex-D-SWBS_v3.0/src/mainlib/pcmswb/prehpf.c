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
*  File: prehpf.c
*  Function: Pre-processing 1-tap high-pass filtering
*            Cut-off (-3dB) frequency is approximately 50 Hz,
*            if the recommended filt_no value is used.
*------------------------------------------------------------------------
*/

#include "pcmswb_common.h"
#include "prehpf.h"

/*****************************/
#ifdef DYN_RAM_CNT
#include "dyn_ram_cnt.h"
#endif
/*****************************/

typedef struct {
  Word16  memx;  /*Q0*/
  Word32  memy;  /*Q14*/
} HPASSMEM;

/* Constructor */
void  *highpass_1tap_iir_const(void)  /* returns pointer to work space */
{
  HPASSMEM *hpmem;

  hpmem = (HPASSMEM *)malloc( sizeof(HPASSMEM) );

  /*****************************/
#ifdef DYN_RAM_CNT
  {
    UWord32 ssize;
    ssize = (UWord32) (SIZE_Ptr);
#ifdef MEM_STT
    ssize += (UWord32) (SIZE_Word16);
    ssize += (UWord32) (SIZE_Word32);
#endif
    DYN_RAM_PUSH(ssize, "dummy");
  }
#endif
  /*****************************/

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
    /*****************************/
#ifdef DYN_RAM_CNT
    DYN_RAM_POP();
#endif
    /*****************************/
  }
}

/* Reset */
void  highpass_1tap_iir_reset(void *ptr)
{
  HPASSMEM *hpmem = (HPASSMEM *)ptr;

#ifdef DYN_RAM_CNT
  DYN_RAM_PUSH((UWord32) SIZE_Ptr, "dummy");
#endif
  if (hpmem != NULL) {
    zero16(sizeof(HPASSMEM)/2, (Word16*)hpmem);
  }
#ifdef DYN_RAM_CNT
  DYN_RAM_POP();
#endif
}

/* Filering */
void  highpass_1tap_iir(
                        Word16  filt_no,  /* (i):   Filter cutoff specification. */
                        /*        Use 5 for 8-kHz input,       */
                        /*            6 for 16-kHz input,      */
                        /*            7 for 32-kHz input       */
                        Word16  n,        /* (i):   Number of samples            */
                        Word16  sigin[],  /* (i):   Input signal (Q0)            */
                        Word16  sigout[], /* (o):   Output signal (Q0)           */
                        void    *ptr      /* (i/o): Work space                   */
                        ) 
{
  Word16      k;
  Word16   sSigpre;  /*Q0*/
  Word32   lAcc;     /*Q14*/
  HPASSMEM *hpmem = (HPASSMEM *)ptr;
  /*****************************/
#ifdef DYN_RAM_CNT
  DYN_RAM_PUSH((UWord32) (2 * SIZE_Word16 + SIZE_Word32 + SIZE_Ptr), "dummy");
#endif
  /*****************************/
  lAcc = hpmem->memy;     move32();
  sSigpre = hpmem->memx;  move16();
  FOR ( k = 0; k < n; k++ )
  {
    /* y[k] = a * y[k-1] + x[k] - x[k-1] */
    lAcc = L_sub( lAcc, L_shr(lAcc, filt_no) );   /* a = 0.9921875 for filt_no=7 */
    /* a = 0.984375  for filt_no=6 */
    /* a = 0.96875   for filt_no=5 */
    lAcc = L_mac( lAcc, 0x2000, *sigin );   /* Q14 */
    lAcc = L_msu( lAcc, 0x2000, sSigpre );  /* Q14 */
    sSigpre  = *sigin++;  move16();
    *sigout++ = round_fx_L_shl(lAcc, 2);     move16();
  }
  hpmem->memx = sSigpre; move16();
  hpmem->memy = lAcc;    move32();
  /*****************************/
#ifdef DYN_RAM_CNT
  DYN_RAM_POP();
#endif
  /*****************************/
}

#ifdef LAYER_STEREO
void  highpass_1tap_iir_stereo(
                        Word16  filt_no,  /* (i):   Filter cutoff specification. */
                        /*        Use 5 for 8-kHz input,       */
                        /*            6 for 16-kHz input,      */
                        /*            7 for 32-kHz input       */
                        Word16  n,        /* (i):   Number of samples            */
                        Word16  sigin[],  /* (i):   Input signal (Q0)            */
                        Word16  sigout[], /* (o):   Output signal (Q0)           */
                        void    *ptr      /* (i/o): Work space                   */
                        ) 
{
  Word16      k;
  Word32   lAcc;     /*Q14*/
  HPASSMEM *hpmem = (HPASSMEM *)ptr;
  Word16 *ptr_sig,*ptr_sigpre;
  /*****************************/
#ifdef DYN_RAM_CNT
  DYN_RAM_PUSH((UWord32) (2 * SIZE_Word16 + SIZE_Word32 + SIZE_Ptr), "dummy");
#endif
  /*****************************/
  /* k = 0 */
  ptr_sig = sigin;
  lAcc = L_sub( hpmem->memy, L_shr(hpmem->memy, filt_no) );   /* a = 0.9921875 for filt_no=7 */
  lAcc = L_mac( lAcc, 0x2000, *ptr_sig);   /* Q14 */
  lAcc = L_msu( lAcc, 0x2000, hpmem->memx);  /* Q14 */
  *sigout++ = round_fx_L_shl(lAcc, 2);    move16();

  ptr_sigpre = ptr_sig++;
  FOR ( k = 1; k < n; k++ )
  {
    /* y[k] = a * y[k-1] + x[k] - x[k-1] */
    lAcc = L_sub( lAcc, L_shr(lAcc, filt_no) );   /* a = 0.9921875 for filt_no=7 */
    /* a = 0.984375  for filt_no=6 */
    /* a = 0.96875   for filt_no=5 */
    lAcc = L_mac( lAcc, 0x2000, *ptr_sig++ );   /* Q14 */
    lAcc = L_msu( lAcc, 0x2000, *ptr_sigpre++ );  /* Q14 */
    *sigout++ = round_fx_L_shl(lAcc, 2);     move16();
  }
  hpmem->memx = *ptr_sigpre; move16();
  hpmem->memy = lAcc;    move32();
  /*****************************/
#ifdef DYN_RAM_CNT
  DYN_RAM_POP();
#endif
  /*****************************/
}
#endif
