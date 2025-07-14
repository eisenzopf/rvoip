/* ITU G.722 3rd Edition (2012-09) */

/*--------------------------------------------------------------------------
 ITU-T G.722 Annex C (ex G.722-SWB-Float) Source Code
 Software Release 1.01 (2012-07)
 (C) 2012 France Telecom, Huawei Technologies, VoiceAge Corp., NTT.
--------------------------------------------------------------------------*/
#ifndef FUNCG722_H
#define FUNCG722_H 200

#include "floatutil.h"

/* DEFINITION FOR SMART PROTOTYPES */
#ifndef ARGS
#if (defined(__STDC__) || defined(VMS) || defined(__DECC)  || defined(MSDOS) || defined(__MSDOS__)) || defined (__CYGWIN__) || defined (_MSC_VER)
#define ARGS(x) x
#else /* Unix: no parameters in prototype! */
#define ARGS(x) ()
#endif
#endif

/* FUNCG722_H */
/* ........................ End of file funcg722.h ......................... */

#if FUNCG722 == SW_FLT

/* Define type for G.722 state structure */
typedef struct
{
  Short          al[3];
  Short          bl[7];
  Short          detl;
  Short          dlt[7]; /* dlt[0]=dlt */
  Short          nbl;
  Short          plt[3]; /* plt[0]=plt */
  Short          rlt[3];
  Short          ah[3];
  Short          bh[7];
  Short          deth;
  Short          dh[7]; /* dh[0]=dh */
  Short          ph[3]; /* ph[0]=ph */
  Short          rh[3];
  Short          sl;
  Short          spl;
  Short          szl;
  Short          nbh;
  Short          sh;
  Short          sph;
  Short          szh;
  Short          qmf_tx_delayx[24];
  Short          qmf_rx_delayx[24];
} g722_state;

Short hsbcod ARGS((Short xh, g722_state *s));
Short lsbdec ARGS((Short ilr, Short mode, g722_state *s));
Short hsbdec ARGS((Short ih, g722_state *s));
Short quantl5b ARGS((Short el, Short detl));

void   hsbdec_reset (g722_state *s);
Short quantl ARGS((Short el, Short detl));
Short quanth ARGS((Short eh, Short deth));
Short filtep ARGS((Short rlt [], Short al []));
Short filtez ARGS((Short dlt [], Short bl []));


Short limit ARGS((Short rl));
Short logsch ARGS((Short ih, Short nbh));
Short logscl ARGS((Short il, Short nbl));
Short scalel ARGS((Short nbpl));
Short scaleh ARGS((Short nbph));
void uppol1 ARGS((Short al [], Short plt []));
void uppol2 ARGS((Short al [], Short plt []));
void upzero ARGS((Short dlt [], Short bl []));
void qmf_tx ARGS((Short xin0, Short xin1, Short *xl, Short *xh, 
                  g722_state *s));
void  qmf_tx_buf (Short **xin, Short *xl, Short *xh, Short **delayx);
void  qmf_rx_buf (Short rl, Short rh, Short **delayx, Short **out);
void  fl_qmf_tx_buf (Short **xin, Short *xl, Short *xh, Short **delayx);
void  fl_qmf_rx_buf (Short rl, Short rh, Short **delayx, Short **out);
void adpcm_adapt_l(Short ind, Short *a, Short *b, Short *d, Short *p, Short *r,
				 Short *nb, Short *det, Short *sz, Short *s);
void adpcm_adapt_h(Short ind, Short *a, Short *b, Short *d, Short *p, Short *r,
				 Short *nb, Short *det, Short *sz, Short *s);

/**************
 *     tables *
 **************/
  extern const Short   misil5b[30];
  extern const Short   q5b[15];
  extern const Short   misih[2][3];
  extern const Short   q2;
  extern const Short   qtab6[64];
  extern const Short   qtab5[32];
  extern const Short   qtab4[16];
  extern const Short   qtab2[4];
  extern const Short   whi[4];
  extern const Short   wli[16];
  extern const Short   ila2[353];
  extern const Short   coef_qmf[24];
  extern const Short * invqbl_tab[4];
  extern const Short   invqbl_shift[4];
  extern const Short * invqbh_tab[4];

  extern const Float fl_coef_qmf[24];

#endif /* FUNCG722_H */
/* ........................ End of file funcg722.h ......................... */

#endif
