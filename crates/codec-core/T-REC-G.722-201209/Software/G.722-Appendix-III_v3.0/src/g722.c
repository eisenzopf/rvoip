/* ITU G.722 3rd Edition (2012-09) */

/* ITU-T G.722 Appendix III                                                      */
/* Version:       1.0                                                            */
/* Revision Date: Nov.02, 2006                                                   */

/*
  ITU-T G.722 Appendix III ANSI-C Source Code

  This software has been developed by Broadcom Corporation. 
  
  Copyright (c)  Broadcom Corporation 2006.  All rights reserved. 

  COPYRIGHT : This file is the property of Broadcom Corporation.  It cannot 
  be copied, used, distributed or modified without obtaining authorization 
  from Broadcom Corporation.  If such authorization is provided, any modified 
  version of the software must contain this header.

  WARRANTIES : This software is made available by  Broadcom Corporation in the 
  hope that it will be useful, but without any warranty, including but not 
  limited to any warranty of non-infringement of any third party intellectual 
  property rights.  Broadcom Corporation is not liable for any direct or 
  indirect consequence  or damages related to the use of the provided software, 
  whether or not foreseeable .
*/

/* Copyright and version information from the original G.722 header:
  ============================================================================

                          U    U   GGG    SSSS  TTTTT
                          U    U  G       S       T
                          U    U  G  GG   SSSS    T
                          U    U  G   G       S   T
                           UUU     GG     SSS     T

                   ========================================
                    ITU-T - USER'S GROUP ON SOFTWARE TOOLS
                   ========================================


       =============================================================
       COPYRIGHT NOTE: This source code, and all of its derivations,
       is subject to the "ITU-T General Public License". Please have
       it  read  in    the  distribution  disk,   or  in  the  ITU-T
       Recommendation G.191 on "SOFTWARE TOOLS FOR SPEECH AND  AUDIO
       CODING STANDARDS". 
       ** This code has  (C) Copyright by CNET Lannion A TSS/CMC **
       =============================================================


MODULE:         USER-LEVEL FUNCTIONS FOR THE UGST G.722 MODULE

ORIGINAL BY:
   Simao Ferraz de Campos Neto
   COMSAT Laboratories                    Tel:    +1-301-428-4516
   22300 Comsat Drive                     Fax:    +1-301-428-9287
   Clarksburg MD 20871 - USA              E-mail: simao.campos@labs.comsat.com
    
   History:
History:
~~~~~~~~
14.Mar.95  v1.0       Released for use ITU-T UGST software package Tool
                      based on the CNET's 07/01/90 version 2.00
01.Jul.95  v2.0       Changed function declarations to work with many compilers;
                      reformated <simao@ctd.comsat.com>
23.Aug.06  v3.0 beta  Updated with STL2005 v2.2 basic operators and G.729.1 methodology
                      <{balazs.kovesi,stephane.ragot}@orange-ft.com>
  ============================================================================
*/

#include <math.h>
#include "g722.h"
#include "stl.h"
#include "g722plc.h"
#if (DMEM)
#include "memutil.h"
#endif
#include "table.h"

#define INV_L_8_FRAMES_P1_8KHZ_int   51 /* Q15 */
#define INV_L_8_FRAMES_P1_8KHZ_frc 3936 /* Q15 of fraction */
#define INV_L_4_FRAMES_P1_8KHZ_int  102 /* Q15 */
#define INV_L_4_FRAMES_P1_8KHZ_frc 2654 /* Q15 of fraction */

void TameData(g722_state *decoder, struct WB_PLC_State *plc);


void g722_reset_encoder(encoder)
g722_state *encoder;
{

  Word16          xl, il;
  Word16          xh, ih, j;

  xl = xh = 0;
#ifdef WMOPS
    move16();
    move16();
#endif
  FOR (j = 0; j < 24; j++)
  {
    encoder->qmf_tx_delayx[j] = 0;
#ifdef WMOPS
    move16();
#endif
  }
  il = lsbcod (xl, 1, encoder);
  ih = hsbcod (xh, 1, encoder);
}
/* .................... end of g722_reset_encoder() ....................... */


Word32 g722_encode(incode,code,read1,encoder)
  short *incode;
  short *code;
  Word32 read1;
  g722_state     *encoder;
{
  /* Encoder variables */
  Word16          xl, il;
  Word16          xh, ih;
  Word16          xin0, xin1;

  /* Auxiliary variables */
  Word32             i;
  Word16          *p_incode;

  /* Divide sample counter by 2 to account for QMF operation */
  read1 = L_shr(read1, 1);

  /* Main loop - never reset */
	p_incode = incode;
#ifdef WMOPS
    move16();
#endif
  FOR (i = 0; i < read1; i++)
  {
    xin1 = *incode++;
    xin0 = *incode++;
#ifdef WMOPS
    move16();
    move16();
#endif

    /* Calculation of the synthesis QMF samples */
    qmf_tx (xin0, xin1, &xl, &xh, encoder);

    /* Call the upper and lower band ADPCM encoders */
    il = lsbcod (xl, 0, encoder);
    ih = hsbcod (xh, 0, encoder);

    /* Mount the output G722 codeword: bits 0 to 5 are the lower-band
     * portion of the encoding, and bits 6 and 7 are the upper-band
     * portion of the encoding */
    code[i] = s_and(add(shl(ih, 6), il), 0xFF);
#ifdef WMOPS
    move16();
#endif
  }

  /* Return number of samples read */
  return(read1);
}
/* .................... end of g722_encode() .......................... */


#ifndef G722ENC
void g722_reset_decoder(decoder)
g722_state *decoder;
{
  Word16          il, ih;
  Word16          j;

  il = ih = 0;
#ifdef WMOPS
    move16();
    move16();
#endif
  FOR (j = 0; j < 24; j++)
  {
    decoder->qmf_rx_delayx[j] = 0;
#ifdef WMOPS
    move16();
#endif
  }
  reset_lsbdec (decoder);
  hsbdec_resetg722(decoder);
}
/* .................... end of g722_reset_decoder() ....................... */

/*-----------------------------------------------------------------------------
 * Function: g722_decode()
 *
 * Description: Frame level G.722 decoder.
 *
 * Inputs:  *code    - array of input indices
 *          mode     - G.722 mode
 *          read1    - frame length
 *          *decoder - G.722 state memory
 *          *plc     - PLC state memory
 *
 * Outputs: returned - number of output samples
 *          *outcode - array of output speech samples
 *          *decoder - G.722 state memory
 *          *plc     - PLC state memory
 *---------------------------------------------------------------------------*/
short g722_decode(code,outcode,mode,read1,decoder, plc)
  short *code;
  short *outcode;
  short mode;
  short read1;
  g722_state     *decoder;
  struct WB_PLC_State *plc;
{
  /* Decoder variables */
   Word16         il, ih;
   Word16         rl, rh;
   Word16         (*plsbdec)(Word16 ilr, Word16 mode, Word16 rs, g722_state *s, Word16 psml);
   Word16         (*phsbdec)(Word16 ih, Word16 rs, g722_state *s, struct WB_PLC_State *plc,
                   Word16 (*pNBPHlpfilter)( struct WB_PLC_State *plc, Word16 inv_frames_int, 
                     Word16 inv_frames_frc, Word16 nbph, Word16 sample), 
                     Word16 (*pDCremoval)(Word16 rh0, Word16 *rhhp, Word16 *rhl),
                     Word16 inv_frame_int, Word16 inv_frame_frc, Word16 sample);

   Word16         inv_frame_int, inv_frame_frc;
   Word16         (*pNBPHfilter) ( struct WB_PLC_State *plc, Word16 inv_frames_int, 
                  Word16 inv_frames_frc, Word16 nbph, Word16 sample);
   Word16         i;  
   Word32         a0;
   Word16         nbph_mean_prev, nbph_trck_abs, nbph_chng_abs;
   Word16         nbpl_mean_prev, mean1_chng_abs, nbpl_trck_abs, nbpl_chng_abs;
   Word16         psml_mean, nbph_trck, nbph_mean, nbph_chng, nbpl_mean1, nbpl_trck, nbpl_mean2, nbpl_chng;
   Word16         psml, psml_start, tt;
   Word16         *pt, safetythres;
   Word16         (*pDCremoval)(Word16 rh0, Word16 *rhhp, Word16 *rhl);  

   /* set up adaptive contraint on pole section of low-band ADPCM predictor */
   psml_start = 3072;
#ifdef WMOPS
   move16();
#endif
   if(sub(plc->psml_mean,3072) < 0)
   {
      psml_start = plc->psml_mean;
#ifdef WMOPS
      move16();
#endif
   }
   psml = 1024;
#ifdef WMOPS
   move16();
#endif
   tt = sub(plc->ngfae, 3);
   if (tt<0)
   {
      psml = psml_start;
#ifdef WMOPS
      move16();
#endif
   }
   psml_start = add(psml_start, (Word16)1024);
   if (tt==0)
   {
      psml = shr(psml_start, 1);
   }

   safetythres = sub(16384, psml);

   /* set up DC removal */
   if (plc->hp_flag != 0)
      pDCremoval = DCremoval;
   if (plc->hp_flag == 0)
      pDCremoval = DCremovalMemUpdate;

   /* set up LP filter on low-band log scale factor */
#ifndef G722DEMO
   IF(plc->ngfae == 0)
   {
      plsbdec= plc_lsbdec;
      phsbdec = plc_hsbdec;

      IF(sub(plc->nbh_mode,2) == 0)
      {
         pNBPHfilter = NBPHlpfilter;
         inv_frame_int = INV_L_8_FRAMES_P1_8KHZ_int;
         inv_frame_frc = INV_L_8_FRAMES_P1_8KHZ_frc;
#ifdef WMOPS
         move16();move16();
#endif
      }
      ELSE IF(sub(plc->nbh_mode,1) == 0)
      {
         pNBPHfilter = NBPHlpfilter;
         inv_frame_int = INV_L_4_FRAMES_P1_8KHZ_int;
         inv_frame_frc = INV_L_4_FRAMES_P1_8KHZ_frc;
#ifdef WMOPS
         move16();move16();
#endif
      }
      ELSE //(plc->nbh_mode == 0)
      {
         pNBPHfilter = NBPHnofilter;
         inv_frame_int = 0;
         inv_frame_frc = 0;
#ifdef WMOPS
         move16();move16();
#endif
      }
   }
   ELSE
#endif /* G722DEMO */
   {
      plsbdec = lsbdec;
      phsbdec = hsbdec;
      pNBPHfilter = NBPHnofilter;
      inv_frame_int = 0;
      inv_frame_frc = 0;
#ifdef WMOPS
      move16();move16();
#endif
      IF(sub(plc->nbh_mode,2) == 0)
      {		
         /* highly stationary nbh prior to loss strong and long LP filter */
         IF(sub(plc->ngfae,7) <= 0)
         {
            pNBPHfilter = NBPHlpfilter;
            inv_frame_int = INV_L_8_FRAMES_P1_8KHZ_int;
            inv_frame_frc = INV_L_8_FRAMES_P1_8KHZ_frc;
#ifdef WMOPS
            move16();move16();
#endif
         }
      }
      ELSE IF(sub(plc->nbh_mode,1) == 0)
      {	
         /* somewhat stationary nbh prior to loss milder and shorter LP filter */
         IF(sub(plc->ngfae,3) <= 0)
         {
            pNBPHfilter = NBPHlpfilter;
            inv_frame_int = INV_L_4_FRAMES_P1_8KHZ_int;
            inv_frame_frc = INV_L_4_FRAMES_P1_8KHZ_frc;
#ifdef WMOPS
            move16();move16();
#endif
         }
      }
   }
   psml_mean = plc->psml_mean;
   nbph_trck = plc->nbph_trck;
   nbph_mean = plc->nbph_mean;
   nbph_chng = plc->nbph_chng;
   nbpl_mean1= plc->nbpl_mean1;
   nbpl_trck = plc->nbpl_trck;
   nbpl_mean2= plc->nbpl_mean2;
   nbpl_chng = plc->nbpl_chng;
#if WMOPS
   move16();move16();move16();move16();move16();move16();move16();move16();
#endif

   /* Decode - reset is never applied here */
   FOR (i = 0; i < read1; i++)
   {
      /* Separate the input G722 codeword: bits 0 to 5 are the lower-band
       * portion of the encoding, and bits 6 and 7 are the upper-band
       * portion of the encoding */
      il = s_and(code[i], 0x3F);	/* 6 bits of low SB */
      ih = s_and(lshr(code[i], 6), 0x03);/* 2 bits of high SB */

      /* Call the upper and lower band ADPCM decoders */
      rl = plsbdec (il, mode, 0, decoder, safetythres);
      rh = phsbdec (ih, 0, decoder, plc, pNBPHfilter, pDCremoval, 
                    inv_frame_int, inv_frame_frc, i);

      a0 = L_mult(30720, psml_mean);
      a0 = L_msu(a0, 2048, abs_s(decoder->al[1]));
      a0 = L_mac(a0, 2048, 16384);
      a0 = L_msu(a0, 2048, decoder->al[2]);
      psml_mean = round(a0);

	   /* Tracking of high-band adaptive log scale factor */
      nbph_mean_prev = nbph_mean;
      a0 = L_mult(W_NBH_TRCK, nbph_trck);
      a0 = L_mac(a0, W_NBH_TRCK_M1, sub(nbph_mean, decoder->nbh));
      nbph_trck = round(a0);
      nbph_trck_abs = abs_s(nbph_trck);
#ifdef WMOPS
      move16();
#endif
      pt = nbphtab;
      if(sub(nbph_trck_abs,1638) >= 0)
         pt+=2;
      if(sub(nbph_trck_abs,3277) >= 0)
         pt+=2;
      if(sub(nbph_trck_abs,4915) >= 0)
         pt+=2;
         
      a0 = L_mult(*pt++, nbph_mean);
      a0 = L_mac(a0, *pt, decoder->nbh);
      nbph_mean = round(a0);

      nbph_chng_abs = abs_s(sub(nbph_mean,nbph_mean_prev));
      a0 = L_shl(L_mult(W_NBH_CHNG_M1, nbph_chng_abs), 8);
      a0 = L_mac(a0,W_NBH_CHNG, nbph_chng);
      nbph_chng = round(a0);

      /* Tracking of low-band adaptive log scale factor */
      nbpl_mean_prev = nbpl_mean1;

      a0 = L_mult(28672, nbpl_mean1);
      a0 = L_mac(a0, 4096, decoder->nbl);
      nbpl_mean1 = round(a0);

      mean1_chng_abs = abs_s(sub(nbpl_mean1,nbpl_mean_prev));
      a0 = L_shl(L_mult(W_NBL_CHNG_M1, mean1_chng_abs), 8);
      a0 = L_mac(a0, W_NBL_CHNG, nbpl_trck);
      nbpl_trck = round(a0);

      nbpl_mean_prev = nbpl_mean2;
#ifdef WMOPS
      move16();move16();
#endif

      nbpl_trck_abs = abs_s(nbpl_trck);

      pt = nbpltab;
      if(sub(nbpl_trck_abs, 3277) >= 0)
         pt += 2;
      if(sub(nbpl_trck_abs, 6554) >= 0)
         pt += 2;
      a0 = L_mult(*pt++, nbpl_mean2);
      a0 = L_mac(a0, *pt, nbpl_mean1);
      nbpl_mean2 = round(a0);
      if(sub(nbpl_trck_abs, 9830) >= 0)
      {
         nbpl_mean2 = nbpl_mean1;
#ifdef WMOPS
         move16();
#endif
      }

      nbpl_chng_abs = abs_s(sub(nbpl_mean2,nbpl_mean_prev));
      a0 = L_shl(L_mult(W_NBL_CHNG_M1, nbpl_chng_abs), 8);
      a0 = L_mac(a0,W_NBL_CHNG, nbpl_chng);
      nbpl_chng = round(a0);

      /* Calculation of output samples from QMF filter */
      qmf_rx (rl, rh, outcode, outcode+1, decoder);
      outcode+=2;
   }
   plc->psml_mean = psml_mean;
   plc->nbph_trck = nbph_trck;
   plc->nbph_mean = nbph_mean;
   plc->nbph_chng = nbph_chng;
   plc->nbpl_mean1= nbpl_mean1;
   plc->nbpl_trck = nbpl_trck;
   plc->nbpl_mean2= nbpl_mean2;
   plc->nbpl_chng = nbpl_chng;
#if WMOPS
   move16();move16();move16();move16();move16();move16();move16();move16();
#endif

    
   /* Return number of samples read */
   return(shl(read1,1));
}
#endif
/* .................... end of g722_decode() .......................... */
