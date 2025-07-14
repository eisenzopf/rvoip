/* ITU G.722 3rd Edition (2012-09) */

/*--------------------------------------------------------------------------
 ITU-T G.722 Annex C (ex G.722-SWB-Float) Source Code
 Software Release 1.01 (2012-07)
 (C) 2012 France Telecom, Huawei Technologies, VoiceAge Corp., NTT.
--------------------------------------------------------------------------*/

#include "floatutil.h"
#include "g722.h"
#include "funcg722.h"
#include "lsbcod_ns.h"
#include "g722_plc.h"
#include "funcg722.h"
#include "hsb_enh.h"
#include "ns_common.h"
#include "bwe.h"


/* G.722 encoder state structure (only used for encoder) */
typedef struct {
  g722_state         g722work;
  fl_noiseshaping_state nswork; 
  fl_noiseshaping_state nswork_enh; 
} fl_g722enc_state;




void *fl_g722_encode_const()
{
  fl_g722enc_state *work = NULL;

  work = (fl_g722enc_state *)malloc(sizeof(fl_g722enc_state));

  if (work != NULL) {
    fl_g722_encode_reset((void *)work);
  }
  return (void *)work;
}



//Floating_ver.
void fl_g722_encode_dest(void *ptr)
{
  fl_g722enc_state *work = (fl_g722enc_state *)ptr;

  if (work != NULL) {
    free(work);
  }
  return;
}


/* void g722_reset_encoder_ns(g722_state *encoder, noise_shaping_state* work) */
void fl_g722_encode_reset(void *ptr)
{
  fl_g722enc_state *work = (fl_g722enc_state *)ptr;
  Short * w16ptr;
  Float *fl_ptr;
  if (work != NULL) {
    w16ptr = (Short*)(&work->g722work);
    zeroS(sizeof(g722_state)/2,w16ptr); 
	fl_ptr = (Float*)(&work->nswork);
	zeroF(sizeof(fl_noiseshaping_state)/4,fl_ptr); /*210 : size of g722enc_state structure in Short (420 bytes)*/
    fl_ptr = (Float*)(&work->nswork_enh);
	zeroF(sizeof(fl_noiseshaping_state)/4,fl_ptr); /*210 : size of g722enc_state structure in Short (420 bytes)*/
	work->g722work.detl = 32; 
    work->g722work.deth = 8; 
    work->nswork.gamma = FL_GAMMA1; 
    work->nswork_enh.gamma = FL_GAMMA1; 
  }
  return;
}


/* .................... end of g722_reset_encoder() ....................... */



void g722_encode(Short mode, Short local_mode, const Short *sig, unsigned char *code,
                 unsigned char *code_enh, Short mode_enh, /* mode_enh = 1 -> high-band enhancement layer */
                 void *ptr, Short wbenh_flag, unsigned short **pBit_wbenh
                 )
{
  fl_g722enc_state *work = (fl_g722enc_state *)ptr;		

  /* Encoder variables */
  /* Auxiliary variables */
  int          i;

  Short          ih_enh[L_FRAME_NB];
  Short         *ptr_enh;
  Short          j;
  Short          xl[L_FRAME_NB], icore[L_FRAME_NB];
  Short          xh[L_FRAME_NB];
  Short          filtmem[L_FRAME_WB + 22];
  Short          *filtptr;
  filtptr = &filtmem[L_FRAME_WB];

  movSS(22, &(work->g722work.qmf_tx_delayx[2]), &(filtmem[L_FRAME_WB])); /* load memory */

  /* Main loop - never reset */
  for (i = 0; i < L_FRAME_NB; i++) {
    /* Calculation of the synthesis QMF samples */
    fl_qmf_tx_buf ((Short **)&sig, &xl[i], &xh[i], &filtptr);
  }
  movSS(22, filtmem, &(work->g722work.qmf_tx_delayx[2])); /*save memory*/

  /* lower band ADPCM encoding */
  fl_lsbcod_buf_ns(xl, icore, &(work->g722work), &(work->nswork), mode, local_mode);

  /* higher band ADPCM encoding */
  fl_hsbcod_buf_ns(xh, icore, ih_enh, &(work->g722work), &(work->nswork_enh),
    mode_enh, wbenh_flag, pBit_wbenh);


  /* Mount the output G722 codeword: bits 0 to 5 are the lower-band
  * portion of the encoding, and bits 6 and 7 are the upper-band
  * portion of the encoding */
  for (i = 0; i < L_FRAME_NB; i++) {
    code[i] = (unsigned char)(icore[i] & 0xFF);
  }

  if( (mode_enh-2) == 0) {
    /* set bytes in the enhancement layer (1 bits/sample -> frame length/8 bytes*/
    ptr_enh = ih_enh;

    for (i = 0; i < L_FRAME_NB>>3; i++) {
      /* initialize to zero */
      code_enh[i] = 0;
      

      /* multiplex */
      for (j=0; j<8; j += (3-mode_enh)) {
        code_enh[i] = (unsigned char)(code_enh[i]+ (*ptr_enh++<<j));
      }
    }
  }

  return;
}



/* .................... end of g722_encode() .......................... */

/* G.722 decoder state structure (only used for decoder) */
typedef struct {
  g722_state         g722work;
  void*              plcwork;
} g722dec_state;


void *g722_decode_const()
{
  g722dec_state *work = NULL;
  work = (g722dec_state *)malloc(sizeof(g722dec_state));
  if (work != NULL) {
    g722_decode_reset((void *)work);
    work->plcwork = G722PLC_init_flt();
  }
  return (void *)work;
}


void g722_decode_dest(void *ptr)
{
  g722dec_state *work = (g722dec_state *)ptr;

  if (work != NULL) {
    G722PLC_clear_flt(work->plcwork);
    free(work);
  }
  return;
}


void g722_decode_reset(void *ptr)
{
  g722dec_state *work = (g722dec_state *)ptr;

  Short * w16ptr;

  if (work != NULL) {
    w16ptr = (Short*)ptr;
    zeroS(sizeof(g722dec_state)/2,w16ptr); /*106 : size of g722dec_state structure in Short (212 bytes)*/
    work->g722work.detl = 32; 
    work->g722work.deth = 8; 

  }
  return;
}



/* .................... end of g722_decode_reset() ....................... */
/*
Word32 G722PLC_decode(short *code, short *outcode, short mode, Short read1,
g722_state *decoder,void *plc_state)
*/
static Short fl_g722_decode1(Short mode, const unsigned char *code,
                           const unsigned char *code_enh, Short mode_enh, Short *rl, Short i, 
                           void *ptr, unsigned short **pBit_wbenh, Short wbenh_flag, Short *enh_no, Float *sum_ma_dh_abs
                           )
{
  Short          il, ih;
  Short          rh, k, nb, ih_enh;

  g722dec_state *work = (g722dec_state *)ptr;				
  g722_state *decoder = &(work->g722work);
  G722PLC_STATE_FLT *plc_state_flt = (G722PLC_STATE_FLT *)(work->plcwork);

  /* Separate the input G722 codeword: bits 0 to 5 are the lower-band
  * portion of the encoding, and bits 6 and 7 are the upper-band
  * portion of the encoding */
  il = code[i] & 0x3F; /* 6 bits of low SB */
  ih = code[i]>> 6; /* 2 bits of high SB */

  ih_enh = 0; 
  if ((mode_enh-2) == 0) {                                      /* i 0 1 2 3 4 5 6 7 8 9 a b c d e f */
    k = i>> 3;                                               /* k 0 0 0 0 0 0 0 0 1 1 1 1 1 1 1 1 mode2*/
    nb = i & 7;                                            /*nb 0 1 2 3 4 5 6 7 0 1 2 3 4 5 6 7 mode2*/   
    ih_enh = (code_enh[k]>>nb) & 1;                      
  }                                                                

  /* Call the upper and lower band ADPCM decoders */
  *rl = lsbdec(il, mode, decoder);
  rh = fl_hsbdec_enh(ih, ih_enh, mode_enh, decoder, i, pBit_wbenh, wbenh_flag, enh_no, sum_ma_dh_abs);

  /* remove-DC filter */
  rh = (Short)Floor(G722PLC_hp_flt(&plc_state_flt->f_mem_hpf_in, &plc_state_flt->f_mem_hpf_out, (Float)rh,
    f_G722PLC_b_hp156, f_G722PLC_a_hp156)+(Float)0.5);
  return(rh);
}



void g722_decode(Short mode, const unsigned char *code,
                 const unsigned char *code_enh, Short mode_enh,
                 int loss_flag, Short *outcode,
                 void *ptr, unsigned short **pBit_wbenh, Short wbenh_flag
                 )
{

  g722dec_state *work = (g722dec_state *)ptr;				
  g722_state *decoder = &(work->g722work);					
  G722PLC_STATE_FLT *plc_state_flt = (G722PLC_STATE_FLT *)(work->plcwork);


  if (loss_flag == 0) {
    /*------ decode good frame ------*/

    /* Decoder variables */
    Short          rl, rh;

    /* Auxiliary variables */
    Short i, j;  
    Float *ptr_l, *ptr_h;
    Short  filtmem[L_FRAME_WB+22];
    Short *filtptr = &filtmem[L_FRAME_WB];
    Float weight;

	Short enh_no;
    Float sum_ma_dh_abs;

    /* shift speech buffers */
    movF(257, &plc_state_flt->f_mem_speech[L_FRAME_NB], plc_state_flt->f_mem_speech); /*shift 5 ms*/
    movF(120, &plc_state_flt->f_mem_speech_hb[L_FRAME_NB], plc_state_flt->f_mem_speech_hb); /*shift 5 ms*/

    ptr_l = &(plc_state_flt->f_mem_speech[257]);	
    ptr_h = &(plc_state_flt->f_mem_speech_hb[120]);	

    /* Decode - reset is never applied here */
    i = 0; 

    movSS(22, &decoder->qmf_rx_delayx[2], &filtmem[L_FRAME_WB]); /* load memory */

    enh_no = NBITS_MODE_R1SM_WBE; 
    sum_ma_dh_abs = 0.; 

	if(plc_state_flt->s_count_crossfade < CROSSFADELEN) /* first good 10 ms, crossfade is needed*/
    {
      for (i = plc_state_flt->s_count_crossfade; i < 20; i++) /*first 20 samples : flat part*/
      {
        /* Separate the input G722 codeword: bits 0 to 5 are the lower-band
        * portion of the encoding, and bits 6 and 7 are the upper-band
        * portion of the encoding */
        rh = fl_g722_decode1(mode, code, code_enh, mode_enh, &rl, i, 
          ptr, pBit_wbenh, wbenh_flag, &enh_no, &sum_ma_dh_abs);
        /* cross-fade samples with PLC synthesis (in lower band only) */
        rl = (Short)(plc_state_flt->f_crossfade_buf[i]);

        /* copy lower and higher band sample */
        *ptr_l++ = (Float)rl; 
        *ptr_h++ = (Float)rh; 

        /* Calculation of output samples from QMF filter */
        fl_qmf_rx_buf (rl, rh, &filtptr, &outcode);
      }

      weight = (Float)0.0166259765; /*546/32768*/  /*first valid frame, after flat part (sample 21)*/
      if(plc_state_flt->s_count_crossfade > 0)/* 2nd valid frame*/
      {
        weight = (Float)0.34991455; /*11466;  21*546*/
      }
      for (; i < plc_state_flt->s_count_crossfade + L_FRAME_NB; i++)
      {
        j =  i - plc_state_flt->s_count_crossfade;
        /* Separate the input G722 codeword: bits 0 to 5 are the lower-band
        * portion of the encoding, and bits 6 and 7 are the upper-band
        * portion of the encoding */
        rh = fl_g722_decode1(mode, code, code_enh, mode_enh, &rl, j, 
          ptr, pBit_wbenh, wbenh_flag, &enh_no, &sum_ma_dh_abs);
        /* cross-fade samples with PLC synthesis (in lower band only) */
		rl = (Short)(rl * weight  + plc_state_flt->f_crossfade_buf[i] * (1- weight));
		weight += (Float)0.0166259765; /*546/32768*/
        
        /* copy lower and higher band sample */
        *ptr_l++ = (Float)rl; 
        *ptr_h++ = (Float)rh; 

        /* Calculation of output samples from QMF filter */
        fl_qmf_rx_buf (rl, rh, &filtptr, &outcode);

      }
      plc_state_flt->s_count_crossfade += L_FRAME_NB; 
    }

    for (; i < L_FRAME_NB; i++)
    {
      /* Separate the input G722 codeword: bits 0 to 5 are the lower-band
      * portion of the encoding, and bits 6 and 7 are the upper-band
      * portion of the encoding */
      rh = fl_g722_decode1(mode, code, code_enh, mode_enh, &rl, i, 
        ptr, pBit_wbenh, wbenh_flag, &enh_no, &sum_ma_dh_abs);
	  /* copy lower and higher band sample */
      *ptr_l++ = (Float)rl; 
      *ptr_h++ = (Float)rh; 

      /* Calculation of output samples from the reference QMF filter */
      fl_qmf_rx_buf (rl, rh, &filtptr, &outcode);
    }

    movSS(22, filtmem, &decoder->qmf_rx_delayx[2]); /* save memory */

    /* set previous bfi to good frame */
    plc_state_flt->s_prev_bfi = 0; 
  }
  else { /* (loss_flag != 0) */

    /*------ decode bad frame ------*/
    G722PLC_conceal_flt(plc_state_flt, outcode, decoder);

    /* set previous bfi to good frame */
    plc_state_flt->s_prev_bfi = 1; 
  }

  return;
}
/* .................... end of g722_decode() .......................... */

