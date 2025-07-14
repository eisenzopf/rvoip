/* ITU G.722 3rd Edition (2012-09) */

/*--------------------------------------------------------------------------
 ITU-T G.722 Annex C (ex G.722-SWB-Float) Source Code
 Software Release 1.01 (2012-07)
 (C) 2012 France Telecom, Huawei Technologies, VoiceAge Corp., NTT.
--------------------------------------------------------------------------*/
#include <math.h>

#include "floatutil.h"
#include "bit_op.h"
#include "ns.h"
#include "hsb_enh.h"
#include "ns_common.h"
#include "bwe.h"
#include "lpctool.h"
#include "funcg722.h"

static Float fl_hsbcod_ldec_l0l1(Short xh, g722_state *s, 
                        Short i, unsigned short **pBit_wbenh, Short *code0, Short *code1, Float *A, 
                        Float *mem1, Float *mem2, Short *enh_no, Float sum, Short wbenh_flag, Short n_cand
                        );


#define AH   s->ah
#define BH   s->bh
#define DETH s->deth
#define DH  s->dh 
#define NBH  s->nbh
#define PH  s->ph 
#define RH  s->rh 
#define SH   s->sh
#define SPH  s->sph 
#define SZH  s->szh

  Short hsbcod_ldec(Short xh, g722_state *s, Short *t, Short *deth, Short *sh)
{
  Short          eh, ih, yh;
  eh = xh- SH;         /* subtra */
  ih = quanth (eh, DETH);

  *deth = DETH;  /* save before update*/
  *sh = SH; 

  adpcm_adapt_h(ih, AH, BH, DH, PH, RH, &NBH, &DETH, &SZH, &SH);
  yh = DH[0]+ *sh; /* output of the previous stage (2bit quantizer) */
  *t = xh- yh;  /*error signal between the input signal and the output of the previous core stage */

  return (ih);
}


static Float fl_hsbcod_ldec_l0l1(Short xh, g722_state *s, 
                        Short i, unsigned short **pBit_wbenh, Short *code0, Short *code1, Float *A, 
                        Float *mem1, Float *mem2, Short *enh_no, Float sum, Short wbenh_flag, Short n_cand
                        )
{
  Short ih, stmp, w16tmp;
  Float dh_abs, t, tw, tmp, err0, err1;
  Short ihr;

  Short dh23, ih23;
  Short deth, sh;
  Short tab_ind, yh;
  Short cand[2];

  if(i == 0)
  {
    sum = -1.f;  /*to force enhancement for the first sample*/
  }

  ih = hsbcod_ldec(xh, s, &stmp, &deth, &sh); /*core coding*/
  t = (Float)stmp;
  *code0 += ih<<6; 

  tw = fl_noise_shaper(A, t, mem1); /*target value, determinated using the noise shaping filter A*/
  mem1[1] = t;  /*EL0 noise shaping memory update*/

  dh23 = DH[0];  /*default 2 bit quantizer*/
  ih23 = ih; 
  tab_ind = 16; 
  dh_abs = 0; 

  if(wbenh_flag > 0)
  {
    if((i+ *enh_no) == L_FRAME_NB )
    {
      sum = -2.f;  /*to force enhancement for the first sample*/
    }
    dh_abs = (Float)abs(DH[0]);
   
    if(  ( (i*dh_abs)> sum)  && (*enh_no > 0))
    {
      /*minimisation of the error between the target value and the possible scalar quantization values*/
      /*ih23: index of the enhancement scalar codeword that minimise the error*/
      w16tmp = 0; 

      if (tw > (tresh_enh[ih] * deth)>>15) /*comparison to the border value*/
      {
        w16tmp = 1; 
      }
      s_PushBit( (unsigned short) w16tmp, pBit_wbenh, 1 );
      ih23 = ( ih<< 1 )| w16tmp ; 
      dh23 = ((long)deth* (long)oq3new[ih23])>>15; /*recontructed value (previous stage + enhancement)*/
      mem1[1] = (Float)t-(dh23- DH[0]); /*update filter memory (with error signal of current stage)*/ 
      *enh_no -= 1; 
      tab_ind = 0; 
    }
  }
  yh = dh23+ sh; /*2 or 3 bit quantizer*/

  if(n_cand > 0)
  {
    ihr = (ih23<<1)+ tab_ind; 
    cand[0] = (((long)deth*(long)oq4_3new[ihr])>>15)- dh23; 
    cand[1] = ( ( (long)deth*(long)oq4_3new[ihr + 1])>>15)- dh23; 

    /* Encode enhancement */
    t = (Float)(xh-yh);
    tw = fl_noise_shaper(A, t, mem2);

    tmp = tw-cand[0];
    err0= tmp* tmp;
    tmp = tw- cand[1];
    err1= tmp*tmp;
    ihr = 0; 
    if(err1< err0) 
    {
      ihr = 1; 
    }
    mem2[1] = (Float)(t- cand[ihr]); 

    *code1 = ihr; 
  }

  return(dh_abs); /* to update sum*/
}



Short fl_hsbdec_enh(Short ih, Short ih_enh, Short mode,  g722_state *s,
                  Short i, unsigned short **pBit_wbenh, Short wbenh_flag, Short *enh_no, Float *sum_ma_dh_abs)
{
  Short dh, rh, yh;
  Float dh_abs;

  Short ih23, dh23, mode23, nshift;
  Short q3bit_flag, sh, deth;

  sh = SH; 
  deth = DETH; 
  adpcm_adapt_h(ih, AH, BH, DH, PH, RH, &NBH, &DETH, &SZH, &SH);

  dh23 = DH[0];  /*default 2 bit quantizer*/
  ih23 = ih; 
  q3bit_flag = 0; 
  if (wbenh_flag > 0)
  {
    dh_abs = (Float)abs(DH[0]);

    if ( (i == 0) || ( ((dh_abs* i)> *sum_ma_dh_abs)  && (*enh_no > 0)) || ((i+ *enh_no)== L_FRAME_NB) )
    {
      ih23 = ih<<1;
      if (GetBit( pBit_wbenh, 1 )>0)
      {
        ih23 = (ih<<1) | 1;
      }
      q3bit_flag = 1; 
      *enh_no = *enh_no - 1;  /*pointer*/
    }
    *sum_ma_dh_abs += dh_abs;  
  }

  nshift = 3-mode; /*determinates number of shifts*/
  mode23 = mode- q3bit_flag; /*mode absolut, if enhancement in EL0, 1 extra bit, lower mode (=higher bitrate) quantizer*/
  ih_enh = (ih23<< nshift)+ (ih_enh & (0x07>>mode)); /*this line is enough without if, works well with mode = 3*/

  dh = ((long)deth* (long)invqbh_tab[mode23][ih_enh])>>15;
  rh = sh+ dh;              /* recons */
  yh = limit( rh );

  return (yh);
}


#undef AH
#undef BH
#undef DETH
#undef DH
#undef NBH
#undef PH
#undef RH
#undef SH
#undef SPH
#undef SZH
/* ........................ End of hsbdec() ........................ */

void
fl_hsbcod_buf_ns(
              const Short sigin[],          /* (i): Input 5-ms signal                     */
              Short code0[],        /* (o): Core-layer bitstream (multiplexed)    */
              Short code1[],        /* (o): LB enh. layer bitstream (multiplexed) */
              g722_state *g722_encoder,
              void *ptr,                     /* (i/o): Pointer to work space               */
              Short mode,
              Short wbenh_flag,
              unsigned short **pBit_wbenh
              )
{
  fl_noiseshaping_state *work = (fl_noiseshaping_state *) ptr;
  Float r[ORD_MP1];             /* Autocorrelations of windowed signal  */
  Float A[ORD_MP1];    /* A0(z) with bandwidth-expansion , not static  */	
  Float   alpha;
  Float  buffer[L_WINDOW];      /* buffer for past decoded signal */
  Float   rc[ORD_M];    /* A0(z) with bandwidth-expansion , not static  */
  Short   i, norm, stable;
  Short enh_no; 
  Float fac;
  Float sum_ma_dh_abs, dh_abs;

  Float  mem_buf1[L_FRAME_NB+ORD_M], mem_buf2[L_FRAME_NB+ORD_M], *memptr1, *memptr2;
  Short n_cand;
  Short w16tmp;

  movF(L_FRAME_NB, work->buffer, buffer);
  movSF(L_FRAME_NB, (Short *)sigin, buffer + L_FRAME_NB);	
  movF(L_FRAME_NB, buffer + L_FRAME_NB, work->buffer);

  /* LP analysis and filter weighting */
  norm = fl_AutocorrNS(buffer, r);
  fl_Levinson(r, rc, &stable, ORD_M, A);

//#define NO_NS
#ifdef NO_NS
#pragma message("#####************* NO_NS HB !!!**********#####")
  A[1] = A[2] = A[3] = A[4] = 0;
#endif

  w16tmp  = norm- MAX_NORM ;
  if (w16tmp>= 0) {
    w16tmp += 1;
	fac = (Float)1./(Float)(1<<(w16tmp ));
	    for (i=1; i<=ORD_M; ++i) {
		  A[i] *= fac;
		  fac *= 0.5f;
    }
  }
  else {
    alpha = -rc[0]; ;       /* rc[0] == -r[1]/r[0]   */
    if (alpha < (Float)-0.984375)   /* r[1]/r[0] < -0.984375 */
    {
      alpha += (Float)1.75;    /* alpha=16*(r[1]/r[0]+1+0.75/16) */
      fl_Weight_a(A, A, FL_GAMMA1*alpha, ORD_M);
    }
	else {
      fl_Weight_a(A, A, FL_GAMMA1, ORD_M);
    }
  }

  /* Compute number of candidates */
  n_cand = (3-mode)<<1; /*mode= 3 : 0; mode = 2 : 2*/
  movF(ORD_M,work->mem_el0,mem_buf1);
  memptr1 = &mem_buf1[3];  

  enh_no = NBITS_MODE_R1SM_WBE; 
  sum_ma_dh_abs = 0; 
  movF(ORD_M,work->mem_t,mem_buf2);
  memptr2 = &mem_buf2[3]; 
  for(i = 0; i < L_FRAME_NB; i++)
  {    
    /* Quantize the current sample
    Delete enhancement bits according to core_mode
    Extract candidates for enhancement */
    dh_abs = fl_hsbcod_ldec_l0l1(sigin[i], g722_encoder,  
      i, pBit_wbenh, code0+i, code1+i, A, 
      memptr1++, memptr2++, &enh_no, sum_ma_dh_abs, wbenh_flag, n_cand
      );
    sum_ma_dh_abs += dh_abs;
  }
  movF_bwd(ORD_M,memptr2,work->mem_t+ORD_MM1);
  movF_bwd(ORD_M,memptr1,work->mem_el0+ORD_MM1);

  return;
}

