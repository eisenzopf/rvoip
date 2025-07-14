/* ITU G.722 3rd Edition (2012-09) */

/*
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
Word16 lsbdec ARGS((Word16 ilr, Word16 mode, Word16 rs, g722_state *s));
Word16 hsbdec ARGS((Word16 ih, Word16 rs, g722_state *s));
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
void uppol1 ARGS((Word16 al [], Word16 plt []));
void uppol2 ARGS((Word16 al [], Word16 plt []));
void upzero ARGS((Word16 dlt [], Word16 bl []));
void qmf_tx ARGS((Word16 xin0, Word16 xin1, Word16 *xl, Word16 *xh, 
		  g722_state *s));
void qmf_rx ARGS((Word16 rl, Word16 rh, Word16 *xout1, Word16 *xout2, 
		  g722_state *s));
void  qmf_rx_buf (Word16 rl, Word16 rh, Word16 **delayx, Word16 **out);

#endif /* FUNCG722_H */
/* ........................ End of file funcg722.h ......................... */
