/* ITU G.722 3rd Edition (2012-09) */

/*--------------------------------------------------------------------------
 ITU-T G.722 Annex C (ex G.722-SWB-Float) Source Code
 Software Release 1.01 (2012-07)
 (C) 2012 France Telecom, Huawei Technologies, VoiceAge Corp., NTT.
--------------------------------------------------------------------------*/
#include <stdio.h>

#include "pcmswb_common.h"
#include "lsbcod_ns.h"
#include "ns.h"
#include "ns_common.h"
#include "floatutil.h"

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
Short lsbcod_ldec(Short xl, g722_state *s, Short local_mode, 
                   Short *yl, Short *detl, Short *dl)
{
  Short          el, il;
  Short mask;

  mask = code_mask[local_mode];

  el = xl- SL;           /* subtra */
  il = quantl5b (el, DETL);

  /* Generate candidates */
  il = il& mask;
  *dl = (Short) ( ( (long)DETL * (long)invqbl_tab[local_mode][il>>invqbl_shift[local_mode]])>>15); 
  *yl = SL+ *dl; 
  *detl = DETL;  

  adpcm_adapt_l(il, AL, BL, DLT, PLT, RLT, &NBL, &DETL, &SZL, &SL);

  /* Return encoded sample */
  return (il);
}


Short fl_lsbcod_ns_core(
                      const Short sigin[],        /* (i): Input 5-ms signal                   */
                      Float * A,              /* (i): Noise shaping filter  */
                      g722_state *adpcm_work,      /* (i/o): Pointer to G.722 work space  */
                      Short local_mode,                  /* (i): G.722 core mode */
                      Short i,
                      Short *sigdec_core,
                      Short *detl,
                      Short *dl,
                      Float ** memptr1)            /* (i): Noise shaping filter memory */
{
  Short idx;
  Float tmp;

  /* Compute signal + noise feedback */
  tmp = fl_noise_shaper(A, sigin[i], (*memptr1)++); /*target value, determinated using the noise shaping filter A*/

  /* Quantize the current sample
  Delete enhancement bits according to local_mode
  Decode locally*/
  idx = lsbcod_ldec((Short)tmp, adpcm_work, local_mode, sigdec_core, detl, dl);
  /* Update the noise-shaping filter memory  */
  **memptr1 = (Float)sigin[i]- (Float)*sigdec_core;  /* **memptr1 is also the target value for the enhancement stage*/

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



void fl_lsbcod_buf_ns(
                   const Short sigin[],        /* (i): Input 5-ms signal                   */
                   Short code0[],              /* (o): G.722 core-layer bitstream (MUX'ed) */
                   g722_state *adpcm_work,      /* (i/o): Pointer to G.722 work space  */
                   fl_noiseshaping_state *ns_work, /* (i/o): Pointer to NS work space */
                   Short mode,                  /* (i): G.722 mode */
                   Short local_mode)            /* (i): local decoding G.722 mode */
{
  Float  r[ORD_MP1];             /* Autocorrelations of windowed signal  */
  Float  A[ORD_MP1];    /* A0(z) with bandwidth-expansion , not static  */

  Float  buffer[L_WINDOW];      /* buffer for past decoded signal */
  Float  rc[ORD_M];    /* A0(z) with bandwidth-expansion , not static  */
  Short  i, norm, stable;
  Short  sigdec_core;
  Short  idx, idx_enh;
  Short  cand[2];
  Float  mem_buf1[L_FRAME_NB+ORD_M], mem_buf2[L_FRAME_NB+ORD_M], *memptr1, *memptr2;

  Short  n_cand, detl, dl;
  Float  tw, tmp;
  Float err0, err1;
  Short itmp, w16tmp;
  Float fac;

  movF(L_FRAME_NB, ns_work->buffer, buffer);
  movSF(L_FRAME_NB, (Short *)sigin, buffer + L_FRAME_NB);	
  movF(L_FRAME_NB, buffer + L_FRAME_NB, ns_work->buffer);	

  /* LP analysis and filter weighting */
  norm = fl_AutocorrNS(buffer, r);
  fl_Levinson(r, rc, &stable, ORD_M, A);

//#define NO_NS
#ifdef NO_NS
#pragma message("#####************* NO_NS LB !!!**********#####")
  A[1] = A[2] = A[3] = A[4] = 0;
#endif

  itmp = norm-MAX_NORM;
  if (itmp >= 0) {
	  fac = (Float)1./(Float)(1<<(itmp+1));
	    for (i=1; i<=ORD_M; ++i) {
		  A[i] *= fac;
		  fac *= 0.5f;
    }
  }
  else {
    if (rc[1]>0.95f) /* 0.95, detect sinusoids */
    {
      ns_work->gamma = 0.0f; 
    }
    fl_Weight_a(A, A, ns_work->gamma, ORD_M);
    ns_work->gamma += FL_GAMMA1S4;
    if (ns_work->gamma> FL_GAMMA1)
    {
      ns_work->gamma = FL_GAMMA1; 
    }
  }

//#define LOAD_NS_FILTER
#ifdef LOAD_NS_FILTER
#pragma message("#####************* LOAD_NS_FILTER LB !!!**********#####")
  {
	  static FILE *fp=NULL;
	  if(fp == NULL) {
		  fp = fopen("c:\\temp\\save_ai_ns", "rb");
		  printf("\n#####************* LOAD_NS_FILTER LB !!!**********#####\n");
	  }
	  fread(&A[1], sizeof(A[1]), 4, fp);
  }
#endif
  n_cand = 0;  
  /* Compute number of candidates */
  itmp = local_mode-mode;
  if (itmp > 0)
  {
    n_cand = 1<<itmp;
  }

  movF(ORD_M,ns_work->mem_wfilter,mem_buf1);
  memptr1 = &mem_buf1[3];
  if (n_cand > 0) { /*n_cand = 2, mode = 1*/
    movF(ORD_M,ns_work->mem_t,mem_buf2);
    memptr2 = &mem_buf2[3];

    for (i = 0; i < L_FRAME_NB; i++) {
      idx = fl_lsbcod_ns_core(sigin, A, adpcm_work, local_mode, i, &sigdec_core, &detl, &dl, &memptr1);    
      code0[i] = idx; 

      /*Extract candidates for enhancement */
      w16tmp = (Short)( ((long)detl* (long)invqbl_tab[mode][idx>>invqbl_shift[mode]])>>15);
      cand[0] = w16tmp- dl;  
      idx++;
      w16tmp = (Short)( ((long)detl* (long)invqbl_tab[mode][idx>>invqbl_shift[mode]])>>15);
      cand[1] = w16tmp- dl;  

      /* Encode enhancement */
      tw = fl_noise_shaper(A, *memptr1, memptr2++); /*target value, determinated using the noise shaping filter A*/

      tmp = tw-(Float)cand[0];
      err0= tmp* tmp;
      tmp = tw-(Float)cand[1];
      err1= tmp *tmp;
      idx_enh = 0; 

      if(err1< err0)
      {
        idx_enh = 1; 
      }
      *memptr2 = *memptr1- (Float)cand[idx_enh];  

      code0[i] += idx_enh; 
    }
    movF_bwd(ORD_M,memptr2,ns_work->mem_t+ORD_MM1);
  }
  else {
    for (i = 0; i < L_FRAME_NB; i++) {
      code0[i] = fl_lsbcod_ns_core(sigin, A, adpcm_work, local_mode, i, &sigdec_core, &detl, &dl, &memptr1); 

    }
  }
  movF_bwd(ORD_M,memptr1,ns_work->mem_wfilter+ORD_MM1);
  return;
}

