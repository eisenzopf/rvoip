/* ITU G.722 3rd Edition (2012-09) */

/*--------------------------------------------------------------------------
 ITU-T Annex B (ex G.722-SWB) Source Code
 Software Release 1.00 (2010-09)
 (C) 2010 France Telecom, Huawei Technologies, NTT, VoiceAge Corp.
--------------------------------------------------------------------------*/

#include "pcmswb_common.h"
#include "lsbcod_ns.h"
#include "ns.h"
#include "ns_common.h"

/*****************************/
#ifdef DYN_RAM_CNT
#include "dyn_ram_cnt.h"
#endif
/*****************************/

#include "lpctool.h"


#define AL   s->al
#define BL   s->bl
#define DETL s->detl
#define DLT  s->dlt
#define NBL  s->nbl
#define PLT  s->plt
#define RLT  s->rlt
#define SL   s->sl
#define SPL  s->spl
#define SZL  s->szl

static
Word16 lsbcod_ldec(Word16 xl, g722_state *s, Word16 local_mode, 
                   Word16 *yl, Word16 *detl, Word16 *dl)
{
  Word16          el, il;
  Word16 mask;
#ifdef DYN_RAM_CNT
  {
    UWord32 ssize = 0;
    ssize += (UWord32) ((0) * SIZE_Ptr);
    ssize += (UWord32) ((3) * SIZE_Word16);
    ssize += (UWord32) ((0) * SIZE_Word32);

    DYN_RAM_PUSH(ssize, "dummy");
  }
#endif

  mask = code_mask[local_mode];
  move16();

  el = sub (xl, SL);           /* subtra */
  il = quantl5b (el, DETL);

  /* Generate candidates */
  il = s_and(il,mask);
  *dl = mult(DETL, invqbl_tab[local_mode][shr(il, invqbl_shift[local_mode])]); move16();
  *yl = add(SL, *dl); move16();
  *detl = DETL; move16(); 

  adpcm_adapt_l(il, AL, BL, DLT, PLT, RLT, &NBL, &DETL, &SZL, &SL);
  /*****************************/
#ifdef DYN_RAM_CNT
  DYN_RAM_POP();
#endif
  /*****************************/

  /* Return encoded sample */
  return (il);
}


Word16 lsbcod_ns_core(
                      const Word16 sigin[],        /* (i): Input 5-ms signal                   */
                      Word16 * A,              /* (i): Noise shaping filter  */
                      g722_state *adpcm_work,      /* (i/o): Pointer to G.722 work space  */
                      Word16 local_mode,                  /* (i): G.722 core mode */
                      Word16 i,
                      Word16 *sigdec_core,
                      Word16 *detl,
                      Word16 *dl,
                      Word16 ** memptr1)            /* (i): Noise shaping filter memory */
{
  Word16 idx, tmp;
#ifdef DYN_RAM_CNT
  {
    UWord32 ssize = 0;
    ssize += (UWord32) ((0) * SIZE_Ptr);
    ssize += (UWord32) ((2) * SIZE_Word16);
    ssize += (UWord32) ((0) * SIZE_Word32);

    DYN_RAM_PUSH(ssize, "dummy");
  }
#endif

  /* Compute signal + noise feedback */
  tmp = noise_shaper(A, sigin[i], (*memptr1)++); /*target value, determinated using the noise shaping filter A*/

  /* Quantize the current sample
  Delete enhancement bits according to local_mode
  Decode locally*/
  idx = lsbcod_ldec(tmp, adpcm_work, local_mode, sigdec_core, detl, dl);
  /* Update the noise-shaping filter memory  */
  **memptr1 = sub(sigin[i], *sigdec_core); move16(); /* **memptr1 is also the targey value for the enhancement stage*/

  /*****************************/
#ifdef DYN_RAM_CNT
  DYN_RAM_POP();
#endif
  /*****************************/
  return(idx);
}

#undef AL
#undef BL
#undef DETL
#undef DLT
#undef NBL
#undef PLT
#undef RLT
#undef SL
#undef SPL
#undef SZL

void lsbcod_buf_ns(
                   const Word16 sigin[],        /* (i): Input 5-ms signal                   */
                   Word16 code0[],              /* (o): G.722 core-layer bitstream (MUX'ed) */
                   g722_state *adpcm_work,      /* (i/o): Pointer to G.722 work space  */
                   noiseshaping_state *ns_work, /* (i/o): Pointer to NS work space */
                   Word16 mode,                  /* (i): G.722 mode */
                   Word16 local_mode)            /* (i): local decoding G.722 mode */
{
  Word16  rh[ORD_MP1];             /* Autocorrelations of windowed signal  */
  Word16  rl[ORD_MP1];   
  Word16  A[ORD_MP1];    /* A0(z) with bandwidth-expansion , not static  */

  Word16  buffer[L_WINDOW];      /* buffer for past decoded signal */
  Word16  rc[ORD_M];    /* A0(z) with bandwidth-expansion , not static  */
  Word16  i, norm, stable;
  Word16  sigdec_core;
  Word16  idx, idx_enh;
  Word16  cand[2];
  Word16  mem_buf1[L_FRAME_NB+ORD_M], mem_buf2[L_FRAME_NB+ORD_M], *memptr1, *memptr2;

  Word16  n_cand, detl, dl;
  Word16  tw, tmp, w16tmp;
  Word32  err0, err1;

#ifdef DYN_RAM_CNT
  {
    UWord32 ssize = 0;
    ssize += (UWord32) ((3*ORD_MP1 + 3*ORD_M + L_WINDOW + 2*L_FRAME_NB + 2 + 12) * SIZE_Word16);
    ssize += (UWord32) ((2) * SIZE_Word32);
    ssize += (UWord32) ((2) * SIZE_Ptr);

    DYN_RAM_PUSH(ssize, "dummy");
  }
#endif
  mov16(L_FRAME_NB, ns_work->buffer, buffer);
  mov16(L_FRAME_NB, (Word16 *)sigin, buffer + L_FRAME_NB);   
  mov16(L_FRAME_NB, buffer + L_FRAME_NB, ns_work->buffer);   

  /* LP analysis and filter weighting */
  norm = AutocorrNS(buffer, rh, rl);
  Levinson(rh, rl, rc, &stable, ORD_M, A);

  w16tmp = sub(norm, MAX_NORM);
  IF (w16tmp >= 0) {
    FOR (i=1; i<=ORD_M; ++i) {
      A[i] = shr(A[i],add(i,w16tmp));
      move16();
    }
  }
  ELSE {
    if (sub(rc[1],31128) > 0) /* 0.95, detect sinusoids */
    {
      ns_work->gamma = 0; move16();
    }
    Weight_a(A, A, ns_work->gamma, ORD_M);
    ns_work->gamma = add(ns_work->gamma, GAMMA1S4);
    if (sub(ns_work->gamma, GAMMA1) > 0)
    {
      ns_work->gamma = GAMMA1; move16();
    }
  }

  n_cand = 0;  move16();

  /* Compute number of candidates */
  tmp = sub(local_mode,mode);
  if (tmp > 0)
  {
    n_cand = shl(1,tmp);
  }

  mov16(ORD_M,ns_work->mem_wfilter,mem_buf1);

  memptr1 = &mem_buf1[3];


  IF (n_cand > 0) { /*n_cand = 2, mode = 1*/
    mov16(ORD_M,ns_work->mem_t,mem_buf2);
    memptr2 = &mem_buf2[3];

    FOR (i = 0; i < L_FRAME_NB; i++) {
      idx = lsbcod_ns_core(sigin, A, adpcm_work, local_mode, i, &sigdec_core, &detl, &dl, &memptr1);    
      code0[i] = idx; move16();

      /*Extract candidates for enhancement */
      w16tmp = mult(detl, invqbl_tab[mode][shr(idx,invqbl_shift[mode])]);
      cand[0] = sub(w16tmp, dl);  move16();
      idx++;
      w16tmp = mult(detl, invqbl_tab[mode][shr(idx,invqbl_shift[mode])]);
      cand[1] = sub(w16tmp, dl);  move16();

      /* Encode enhancement */
      tw = noise_shaper(A, *memptr1, memptr2++); /*target value, determinated using the noise shaping filter A*/

      w16tmp = sub(tw, cand[0]);
      err0= L_mult(w16tmp, w16tmp);
      w16tmp = sub(tw, cand[1]);
      err1= L_mult(w16tmp, w16tmp);

      idx_enh = 0; move16();

      if(L_sub(err1, err0) < 0)
      {
        idx_enh = 1; move16();
      }

      *memptr2 = sub(*memptr1, cand[idx_enh]); move16(); 

      code0[i] = add(code0[i], idx_enh); move16();

    }
    mov16_bwd(ORD_M,memptr2,ns_work->mem_t+ORD_MM1);
  }
  ELSE {
    FOR (i = 0; i < L_FRAME_NB; i++) {
      code0[i] = lsbcod_ns_core(sigin, A, adpcm_work, local_mode, i, &sigdec_core, &detl, &dl, &memptr1); move16();

    }
  }
  mov16_bwd(ORD_M,memptr1,ns_work->mem_wfilter+ORD_MM1);

  /*****************************/
#ifdef DYN_RAM_CNT
  DYN_RAM_POP();
#endif
  /*****************************/
  return;
}

