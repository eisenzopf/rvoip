/* ITU G.722 3rd Edition (2012-09) */

/*--------------------------------------------------------------------------
 ITU-T Annex B (ex G.722-SWB) Source Code
 Software Release 1.00 (2010-09)
 (C) 2010 France Telecom, Huawei Technologies, NTT, VoiceAge Corp.
--------------------------------------------------------------------------*/

#include "bit_op.h"
#include "ns.h"
#include "hsb_enh.h"
#include "ns_common.h"
#include "bwe.h"
#include "lpctool.h"
#include "funcg722.h"

/*****************************/
#ifdef DYN_RAM_CNT
#include "dyn_ram_cnt.h"
#endif
/*****************************/

#ifdef WMOPS
  extern short           Id;
#endif

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
Word16 hsbcod_ldec(Word16 xh, g722_state *s, Word16 *t, Word16 *deth, Word16 *sh)
{
  Word16          eh, ih, yh;
#ifdef DYN_RAM_CNT
  {
    UWord32 ssize = 0;
    ssize += (UWord32) ((0) * SIZE_Ptr);
    ssize += (UWord32) ((3) * SIZE_Word16);
    ssize += (UWord32) ((0) * SIZE_Word32);

    DYN_RAM_PUSH(ssize, "dummy");
  }
#endif

  eh = sub (xh, SH);         /* subtra */
  ih = quanth (eh, DETH);

  *deth = DETH; move16(); /* save before update*/
  *sh = SH; move16();

  adpcm_adapt_h(ih, AH, BH, DH, PH, RH, &NBH, &DETH, &SZH, &SH);
  yh = add(DH[0], *sh); /* output of the previous stage (2bit quantizer) */
  *t = sub(xh, yh);  move16();/*error signal between the input signal and the output of the previous core stage */

#ifdef DYN_RAM_CNT
  DYN_RAM_POP();
#endif
  return (ih);
}

Word16 hsbcod_ldec_l0l1(Word16 xh, g722_state *s, 
                        Word16 i, UWord16 **pBit_wbenh, Word16 *code0, Word16 *code1, Word16 *A, 
                        Word16 *mem1, Word16 *mem2, Word16 *enh_no, Word32 i_sum, Word16 wbenh_flag, Word16 n_cand
                        )
{
  Word16 ih, tw;
  Word16 ihr;
  Word16 i_abs;

  Word16 dh23, ih23;
  Word16 t, deth, sh;
  Word16 tab_ind, yh;
  Word16 cand[2], w16tmp;
  Word32 err0, err1;
#ifdef DYN_RAM_CNT
  {
    UWord32 ssize = 0;
    ssize += (UWord32) ((0) * SIZE_Ptr);
    ssize += (UWord32) ((14) * SIZE_Word16);
    ssize += (UWord32) ((2) * SIZE_Word32);

    DYN_RAM_PUSH(ssize, "dummy");
  }
#endif

  if(i == 0)
  {
    i_sum = -1;  move16();/*to force enhancement for the first sample*/
  }

  ih = hsbcod_ldec(xh, s, &t, &deth, &sh); /*core coding*/
  *code0 = add(*code0, shl(ih,6)); move16();

  tw = noise_shaper(A, t, mem1); /*target value, determinated using the noise shaping filter A*/
  mem1[1] = t; move16(); /*EL0 noise shaping memory update*/

  dh23 = DH[0]; move16(); /*default 2 bit quantizer*/
  ih23 = ih; move16();
  tab_ind = 16; move16();
  i_abs = 0; move16();

  IF(wbenh_flag > 0)
  {
    if(sub(add(i, *enh_no), L_FRAME_NB) == 0)
    {
      i_sum = -2;  move16();/*to force enhancement for the first sample*/
    }
    i_abs = abs_s(DH[0]);

    test();
    IF ((L_sub(L_mult0(i_abs, i), i_sum) > 0) && (*enh_no > 0))
    {
      /*minimisation of the error between the target value and the possible scalar quantization values*/
      /*ih23: index of the enhancement scalar codeword that minimise the error*/
      w16tmp = 0; move16();
      if (sub(tw, mult(tresh_enh[ih], deth)) > 0) /*comparison to the border value*/
      {
        w16tmp = 1; move16();
      }
      PushBit( (UWord16) w16tmp, pBit_wbenh, 1 );
      ih23 = s_or( shl( ih, 1 ), w16tmp ); 
      dh23 = mult (deth, oq3new[ih23]); /*recontructed value (previous stage + enhancement)*/
      mem1[1] = sub(t, sub(dh23, DH[0])); /*update filter memory (with error signal of current stage)*/ move16();
      *enh_no = sub(*enh_no, 1); move16();
      tab_ind = 0; move16();
    }
  }
  yh = add(dh23, sh); /*2 or 3 bit quantizer*/

  IF(n_cand > 0)
  {
    ihr = add(shl(ih23,1), tab_ind); 
    cand[0] = sub(mult (deth, oq4_3new[ihr]), dh23); move16();
    cand[1] = sub(mult (deth, oq4_3new[ihr + 1]), dh23); move16();

    /* Encode enhancement */
    t = sub(xh, yh);
    tw = noise_shaper(A, t, mem2);

    w16tmp = sub(tw, cand[0]);
    err0= L_mult(w16tmp, w16tmp);
    w16tmp = sub(tw, cand[1]);
    err1= L_mult(w16tmp, w16tmp);
    ihr = 0; move16();
    if(L_sub(err1, err0) < 0)
    {
      ihr = 1; move16();
    }
    mem2[1] = sub(t, cand[ihr]); move16();

    *code1 = ihr; move16();
  }
  /*****************************/
#ifdef DYN_RAM_CNT
  DYN_RAM_POP();
#endif
  /*****************************/

  return(i_abs); /* to update i_sum*/
}


Word16 hsbdec_enh(Word16 ih, Word16 ih_enh, Word16 mode,  g722_state *s,
                  Word16 i, UWord16 **pBit_wbenh, Word16 wbenh_flag, Word16 *enh_no, Word32 *i_sum)
{
  Word16 dh, rh, yh;
  Word16 i_abs;

  Word16 ih23, dh23, mode23, nshift;
  Word16 q3bit_flag, sh, deth;
#ifdef DYN_RAM_CNT
  {
    UWord32 ssize = 0;
    ssize += (UWord32) ((0) * SIZE_Ptr);
    ssize += (UWord32) ((11) * SIZE_Word16);
    ssize += (UWord32) ((0) * SIZE_Word32);

    DYN_RAM_PUSH(ssize, "dummy");
  }
#endif

  sh = SH; move16();
  deth = DETH; move16();
  adpcm_adapt_h(ih, AH, BH, DH, PH, RH, &NBH, &DETH, &SZH, &SH);

  dh23 = DH[0]; move16(); /*default 2 bit quantizer*/
  ih23 = ih; move16();
  q3bit_flag = 0; move16();
  IF (wbenh_flag > 0)
  {
    i_abs = abs_s(DH[0]);

    test();test();test();
    IF ((i == 0) || ((L_sub(L_mult0(i_abs, i), *i_sum) > 0) && (*enh_no > 0)) || (sub(add(i, *enh_no), L_FRAME_NB) == 0))
    {
      ih23 = shl(ih,1);
      IF (GetBit( pBit_wbenh, 1 )>0)
      {
        ih23 = s_or(shl(ih,1), 1);
      }
      q3bit_flag = 1; move16();
      *enh_no = sub(*enh_no, 1); move16(); /*pointer*/
    }
    *i_sum = L_mac0(*i_sum, i_abs, 1); move16(); /*pointer*/
  }

  nshift = sub(3,mode); /*determinates number of shifts*/
  mode23 = sub(mode, q3bit_flag); /*mode absolut, if enhancement in EL0, 1 extra bit, lower mode (=higher bitrate) quantizer*/
  ih_enh = add(shl(ih23, nshift), s_and(ih_enh, shr(0x07, mode))); /*this line is enough without if, works well with mode = 3*/

  dh = mult(deth, invqbh_tab[mode23][ih_enh]);
  rh = add (sh, dh);              /* recons */
  yh = limit( rh );

  /*****************************/
#ifdef DYN_RAM_CNT
  DYN_RAM_POP();
#endif
  /*****************************/

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
hsbcod_buf_ns(
              const Word16 sigin[],          /* (i): Input 5-ms signal                     */
              Word16 code0[],        /* (o): Core-layer bitstream (multiplexed)    */
              Word16 code1[],        /* (o): LB enh. layer bitstream (multiplexed) */
              g722_state *g722_encoder,
              void *ptr,                     /* (i/o): Pointer to work space               */
              Word16 mode,
              Word16 wbenh_flag,
              UWord16 **pBit_wbenh
              )
{
  noiseshaping_state *work = (noiseshaping_state *) ptr;
  Word16  rh[ORD_MP1];             /* Autocorrelations of windowed signal  */
  Word16  rl[ORD_MP1];		
  Word16  A[ORD_MP1];    /* A0(z) with bandwidth-expansion , not static  */	
  Word16   sAlpha;
  Word16  buffer[L_WINDOW];      /* buffer for past decoded signal */
  Word16  rc[ORD_M];    /* A0(z) with bandwidth-expansion , not static  */
  Word16   i, norm, stable;
  Word16 enh_no, i_abs; 

  Word32 i_sum; 
  Word16  mem_buf1[L_FRAME_NB+ORD_M], mem_buf2[L_FRAME_NB+ORD_M], *memptr1, *memptr2;
  Word16 n_cand;
  Word16 w16tmp;


#ifdef DYN_RAM_CNT
  {
    UWord32 ssize = 0;
    ssize += (UWord32) ((3) * SIZE_Ptr);
    ssize += (UWord32) ((3*ORD_MP1 + 3*ORD_M + L_WINDOW + 2*L_FRAME_NB + 8) * SIZE_Word16);

    ssize += (UWord32) ((1) * SIZE_Word32);

    DYN_RAM_PUSH(ssize, "dummy");
  }
#endif

  mov16(L_FRAME_NB, work->buffer, buffer);
  mov16(L_FRAME_NB, (Word16 *)sigin, buffer + L_FRAME_NB);	
  mov16(L_FRAME_NB, buffer + L_FRAME_NB, work->buffer);

  /* LP analysis and filter weighting */
  norm = AutocorrNS(buffer, rh, rl);
  Levinson(rh, rl, rc, &stable, ORD_M, A);

  w16tmp  = sub(norm, MAX_NORM) ;
  IF (w16tmp>= 0) {
    w16tmp = add(w16tmp, 1);
    FOR (i=1; i<=ORD_M; ++i) {
      A[i] = shr(A[i],w16tmp);
      move16();
      w16tmp = add(w16tmp, 1);
    }
  }

  ELSE {
    sAlpha = negate(rc[0]); ;       /* rc[0] == -r[1]/r[0]   */
    IF (sub (sAlpha, -32256) < 0)   /* r[1]/r[0] < -0.984375 */
    {
      sAlpha = add (sAlpha, 32767);
      sAlpha = add (sAlpha, 1536);
      sAlpha = shl (sAlpha, 4);     /* alpha=16*(r[1]/r[0]+1+0.75/16) */
      Weight_a(A, A, mult_r(GAMMA1, sAlpha), ORD_M);
    }
    ELSE {
      Weight_a(A, A, GAMMA1, ORD_M);
    }
  }

  /* Compute number of candidates */
  n_cand = shl(sub(3,mode),1); /*mode= 3 : 0; mode = 2 : 2*/
  mov16(ORD_M,work->mem_el0,mem_buf1);
  memptr1 = &mem_buf1[3];  

  enh_no = NBITS_MODE_R1SM_WBE; move16();
  i_sum = 0; move32();
  mov16(ORD_M,work->mem_t,mem_buf2);
  memptr2 = &mem_buf2[3]; 

  FOR(i = 0; i < L_FRAME_NB; i++)
  {    
    /* Quantize the current sample
    Delete enhancement bits according to core_mode
    Extract candidates for enhancement */
    i_abs = hsbcod_ldec_l0l1(sigin[i], g722_encoder,  
      i, pBit_wbenh, code0+i, code1+i, A, 
      memptr1++, memptr2++, &enh_no, i_sum, wbenh_flag, n_cand
      );
    i_sum = L_mac0(i_sum, i_abs, 1);
  }
  mov16_bwd(ORD_M,memptr2,work->mem_t+ORD_MM1);
  mov16_bwd(ORD_M,memptr1,work->mem_el0+ORD_MM1);

  /*****************************/
#ifdef DYN_RAM_CNT
  DYN_RAM_POP();
#endif
  /*****************************/
}
