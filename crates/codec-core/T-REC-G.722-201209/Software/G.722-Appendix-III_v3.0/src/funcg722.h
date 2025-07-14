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
   File: FUNCG722.H                                  v3.0 beta - 23/Aug/2006
  ============================================================================

			UGST/ITU-T G722 MODULE

		      GLOBAL FUNCTION PROTOTYPES

		 --------------------------------------
		  (C) Copyright CNET Lannion A TSS/CMC
		 --------------------------------------

   Original Author:
   
    J-P PETIT 
    CNET - Centre Lannion A
    LAA-TSS                         Tel: +33-96-05-39-41
    Route de Tregastel - BP 40      Fax: +33-96-05-13-16
    F-22301 Lannion CEDEX           Email: petitjp@lannion.cnet.fr
    FRANCE
    
   History:
   14.Mar.95    v1.0    Released for use ITU-T UGST software package Tool
                        based on the CNET's 07/01/90 version 2.00
   01.Jul.95    v2.0    Smart prototypes that work with many compilers; 
                        reformated; state variable structure added. 
  ============================================================================
*/

#ifndef FUNCG722_H
#define FUNCG722_H 200

#ifdef OLD_WAY
Word16 scalel();

Word16 quantl();

void uppol2();

void uppol1();

void upzero();

Word16 filtep();

Word16 filtez();

Word16 invqal();

Word16 limit();

Word16 invqbl();

Word16 invqah();

Word16 logsch();

Word16 scaleh();

Word16 quanth();

Word16 hsbdec();

Word16 hsbcod();

Word16 lsbdec();

Word16 lsbcod();

Word16 logscl();

void qmf_tx();

void qmf_rx();
#endif

/* DEFINITION FOR SMART PROTOTYPES */
#ifndef ARGS
#if (defined(__STDC__) || defined(VMS) || defined(__DECC)  || defined(MSDOS) || defined(__MSDOS__)) || defined (__CYGWIN__) || defined (_MSC_VER)
#define ARGS(x) x
#else /* Unix: no parameters in prototype! */
#define ARGS(x) ()
#endif
#endif


Word16 lsbcod ARGS((Word16 xl, Word16 rs, g722_state *s));
Word16 hsbcod ARGS((Word16 xh, Word16 rs, g722_state *s));
Word16 lsbdec ARGS((Word16 ilr, Word16 mode, Word16 rs, g722_state *s, Word16 psml));
Word16 hsbdec (Word16 ih, Word16 rs, g722_state *s, struct WB_PLC_State *plc,
                   Word16 (*pNBPHlpfilter)( struct WB_PLC_State *plc, Word16 inv_frames_int, 
                     Word16 inv_frames_frc, Word16 nbph, Word16 sample), 
                     Word16 (*pDCremoval)(Word16 rh0, Word16 *rhhp, Word16 *rhl),
                     Word16 inv_frame_int, Word16 inv_frame_frc, Word16 sample);
Word16 quantl ARGS((Word16 el, Word16 detl));
Word16 quanth ARGS((Word16 eh, Word16 deth));
Word16 filtep ARGS((Word16 rlt [], Word16 al []));
Word16 filtez ARGS((Word16 dlt [], Word16 bl []));
Word16 invqal ARGS((Word16 il, Word16 detl));
Word16 invqbl ARGS((Word16 ilr, Word16 detl, Word16 mode));
Word16 invqah ARGS((Word16 ih, Word16 deth));
Word16 limit ARGS((Word16 rl));
Word16 logsch ARGS((Word16 ih, Word16 nbh));
Word16 logscl ARGS((Word16 il, Word16 nbl));
Word16 scalel ARGS((Word16 nbpl));
Word16 scaleh ARGS((Word16 nbph));
void uppol1 ARGS((Word16 al [], Word16 plt [], Word16 minsafety));
void uppol2 ARGS((Word16 al [], Word16 plt []));
void upzero ARGS((Word16 dlt [], Word16 bl []));
void qmf_tx ARGS((Word16 xin0, Word16 xin1, Word16 *xl, Word16 *xh, 
		  g722_state *s));
void qmf_rx ARGS((Word16 rl, Word16 rh, Word16 *xout1, Word16 *xout2, 
		  g722_state *s));

#endif /* FUNCG722_H */
/* ........................ End of file funcg722.h ......................... */
