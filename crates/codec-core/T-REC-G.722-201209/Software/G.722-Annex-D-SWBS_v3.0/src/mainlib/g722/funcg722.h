/* ITU G.722 3rd Edition (2012-09) */

/*--------------------------------------------------------------------------
 ITU-T Annex B (ex G.722-SWB) Source Code
 Software Release 1.00 (2010-09)
 (C) 2010 France Telecom, Huawei Technologies, NTT, VoiceAge Corp.
--------------------------------------------------------------------------*/

/*
  ============================================================================
   File: FUNCG722.H                                  v3.0 beta - 23/Aug/2006
  ============================================================================

                UGST/ITU-T G722 MODULE

              GLOBAL FUNCTION PROTOTYPES

         --------------------------------------
          (C) Copyright CNET Lannion A TSS/CMC
              (now France Telecom-Orange)
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
   23.Aug.06 v3.0 beta  Updated with STL2005 v2.2 basic operators and G.729.1
                        methodology <{balazs.kovesi,stephane.ragot}@orange-ft.com>
  ============================================================================
*/
#ifndef FUNCG722_H
#define FUNCG722_H 200

#include "stl.h"

/* DEFINITION FOR SMART PROTOTYPES */
#ifndef ARGS
#if (defined(__STDC__) || defined(VMS) || defined(__DECC)  || defined(MSDOS) || defined(__MSDOS__)) || defined (__CYGWIN__) || defined (_MSC_VER)
#define ARGS(x) x
#else /* Unix: no parameters in prototype! */
#define ARGS(x) ()
#endif
#endif

/* Define type for G.722 state structure */
typedef struct
{
  Word16          al[3];
  Word16          bl[7];
  Word16          detl;
  Word16          dlt[7]; /* dlt[0]=dlt */
  Word16          nbl;
  Word16          plt[3]; /* plt[0]=plt */
  Word16          rlt[3];
  Word16          ah[3];
  Word16          bh[7];
  Word16          deth;
  Word16          dh[7]; /* dh[0]=dh */
  Word16          ph[3]; /* ph[0]=ph */
  Word16          rh[3];
  Word16          sl;
  Word16          spl;
  Word16          szl;
  Word16          nbh;
  Word16          sh;
  Word16          sph;
  Word16          szh;
  Word16          qmf_tx_delayx[24];
  Word16          qmf_rx_delayx[24];
} g722_state;

Word16 hsbcod ARGS((Word16 xh, g722_state *s));
Word16 lsbdec ARGS((Word16 ilr, Word16 mode, g722_state *s));
Word16 hsbdec ARGS((Word16 ih, g722_state *s));
Word16 quantl5b ARGS((Word16 el, Word16 detl));

void   hsbdec_reset (g722_state *s);
Word16 quantl ARGS((Word16 el, Word16 detl));
Word16 quanth ARGS((Word16 eh, Word16 deth));
Word16 filtep ARGS((Word16 rlt [], Word16 al []));
Word16 filtez ARGS((Word16 dlt [], Word16 bl []));


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
void  qmf_tx_buf (Word16 **xin, Word16 *xl, Word16 *xh, Word16 **delayx);
void  qmf_rx_buf (Word16 rl, Word16 rh, Word16 **delayx, Word16 **out);
void adpcm_adapt_l(Word16 ind, Word16 *a, Word16 *b, Word16 *d, Word16 *p, Word16 *r,
                   Word16 *nb, Word16 *det, Word16 *sz, Word16 *s);
void adpcm_adapt_h(Word16 ind, Word16 *a, Word16 *b, Word16 *d, Word16 *p, Word16 *r,
                   Word16 *nb, Word16 *det, Word16 *sz, Word16 *s);

/**************
 *     tables *
 **************/
  extern const Word16   misil5b[30];
  extern const Word16   q5b[15];
  extern const Word16   misih[2][3];
  extern const Word16   q2;
  extern const Word16   qtab6[64];
  extern const Word16   qtab5[32];
  extern const Word16   qtab4[16];
  extern const Word16   qtab2[4];
  extern const Word16   whi[4];
  extern const Word16   wli[16];
  extern const Word16   ila2[353];
  extern const Word16   coef_qmf[24];
  extern const Word16 * invqbl_tab[4];
  extern const Word16   invqbl_shift[4];
  extern const Word16 * invqbh_tab[4];

#endif /* FUNCG722_H */
/* ........................ End of file funcg722.h ......................... */
