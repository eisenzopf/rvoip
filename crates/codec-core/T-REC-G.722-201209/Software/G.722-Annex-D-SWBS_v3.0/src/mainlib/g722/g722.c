/* ITU G.722 3rd Edition (2012-09) */

/*-----------------------------------------------------------------------------------
 ITU-T G.722-SWBS / G.722 Annex D - Reference C code for fixed-point implementation          
 Version 1.0
 Copyright (c) 2012,
 Huawei Technologies
-----------------------------------------------------------------------------------*/

/*--------------------------------------------------------------------------
 ITU-T Annex B (ex G.722-SWB) Source Code
 Software Release 1.00 (2010-09)
 (C) 2010 France Telecom, Huawei Technologies, NTT, VoiceAge Corp.
--------------------------------------------------------------------------*/

/*
ITU-T G.722 PLC Appendix IV   ANSI-C Source Code
Copyright (c) 2006-2007
France Telecom
*/

#include "g722.h"
#include "funcg722.h"
#include "lsbcod_ns.h"
#include "g722_plc.h"
#include "funcg722.h"
#include "hsb_enh.h"
#include "ns_common.h"
#include "bwe.h"

/*****************************/
#ifdef DYN_RAM_CNT
#include "dyn_ram_cnt.h"
#endif
/*****************************/

/* G.722 encoder state structure (only used for encoder) */
typedef struct {
  g722_state         g722work;
  noiseshaping_state nswork; 
  noiseshaping_state nswork_enh; 
} g722enc_state;

void *g722_encode_const()
{
  g722enc_state *work = NULL;

  work = (g722enc_state *)malloc(sizeof(g722enc_state));

  if (work != NULL) {
    g722_encode_reset((void *)work);
#ifdef DYN_RAM_CNT
    {
      UWord32 ssize = 0;
#ifdef MEM_STT
      ssize += (UWord32) (sizeof(g722enc_state));
#endif
      DYN_RAM_PUSH(ssize, "dummy");
    }
#endif
  }
  return (void *)work;
}

void g722_encode_dest(void *ptr)
{
  g722enc_state *work = (g722enc_state *)ptr;

  if (work != NULL) {
    free(work);
    /*****************************/
#ifdef DYN_RAM_CNT
    DYN_RAM_POP();
#endif
    /*****************************/
  }
  return;
}

/* void g722_reset_encoder_ns(g722_state *encoder, noise_shaping_state* work) */
void g722_encode_reset(void *ptr)
{
  g722enc_state *work = (g722enc_state *)ptr;
  Word16 * w16ptr;

#ifdef DYN_RAM_CNT
  {
    UWord32 ssize = 0;
    ssize += (UWord32) ((2) * SIZE_Ptr);
    ssize += (UWord32) ((0) * SIZE_Word16);
    ssize += (UWord32) ((0) * SIZE_Word32);

    DYN_RAM_PUSH(ssize, "dummy");
  }
#endif
  if (work != NULL) {
    w16ptr = (Word16*)ptr;
    zero16(sizeof(g722enc_state)/2,w16ptr); /*210 : size of g722enc_state structure in Word16 (420 bytes)*/
    work->g722work.detl = 32; move16();
    work->g722work.deth = 8; move16();
    work->nswork.gamma = GAMMA1; move16();
    work->nswork_enh.gamma = GAMMA1; move16();
  }
  /*****************************/
#ifdef DYN_RAM_CNT
  DYN_RAM_POP();
#endif
  /*****************************/
  return;
}
/* .................... end of g722_reset_encoder() ....................... */

void g722_encode(Word16 mode, Word16 local_mode, const Word16 *sig, unsigned char *code,
                 unsigned char *code_enh, Word16 mode_enh, /* mode_enh = 1 -> high-band enhancement layer */
                 void *ptr, Word16 wbenh_flag, UWord16 **pBit_wbenh
                 )
{
  g722enc_state *work = (g722enc_state *)ptr;      

  /* Encoder variables */
  /* Auxiliary variables */
  Word32          i;

  Word16          ih_enh[L_FRAME_NB];
  Word16         *ptr_enh;
  Word16          j;
  Word16          xl[L_FRAME_NB], icore[L_FRAME_NB];
  Word16          xh[L_FRAME_NB];
  Word16          *filtmem;
  Word16          *filtptr;
  filtmem = calloc(L_FRAME_WB + 22, sizeof(*filtmem));
  filtptr = &filtmem[L_FRAME_WB];


#ifdef DYN_RAM_CNT
  {
    UWord32 ssize = 0;
    ssize += (UWord32) ((4) * SIZE_Ptr);
    ssize += (UWord32) ((4*L_FRAME_NB + 1) * SIZE_Word16);
    ssize += (UWord32) ((1) * SIZE_Word32);

    DYN_RAM_PUSH(ssize, "dummy");
  }
  DYN_RAM_PUSH(L_FRAME_WB + 22, "dummy"); /*for filtmem, pop after free() */
#endif

  mov16(22, &(work->g722work.qmf_tx_delayx[2]), &(filtmem[L_FRAME_WB])); /* load memory */

  /* Main loop - never reset */
  FOR (i = 0; i < L_FRAME_NB; i++) {
    /* Calculation of the synthesis QMF samples */
    qmf_tx_buf ((Word16 **)&sig, &xl[i], &xh[i], &filtptr);
  }
  mov16(22, filtmem, &(work->g722work.qmf_tx_delayx[2])); /*save memory*/
  free(filtmem);
  /*****************************/
#ifdef DYN_RAM_CNT
  DYN_RAM_POP();
#endif
  /*****************************/

  /* lower band ADPCM encoding */
  lsbcod_buf_ns(xl, icore, &(work->g722work), &(work->nswork), mode, local_mode);

  /* higher band ADPCM encoding */
  hsbcod_buf_ns(xh, icore, ih_enh, &(work->g722work), &(work->nswork_enh),
    mode_enh, wbenh_flag, pBit_wbenh
    );


  /* Mount the output G722 codeword: bits 0 to 5 are the lower-band
  * portion of the encoding, and bits 6 and 7 are the upper-band
  * portion of the encoding */
  FOR (i = 0; i < L_FRAME_NB; i++) {
    code[i] = (unsigned char)s_and(icore[i], 0xFF);
    move16();
  }

  IF (sub(mode_enh,2) == 0) {
    /* set bytes in the enhancement layer (1 bits/sample -> frame length/8 bytes*/
    ptr_enh = ih_enh;

    FOR (i = 0; i < 5; i++) {
      /* initialize to zero */
      code_enh[i] = (unsigned char)(*ptr_enh++);
      move16();

      /* multiplex */
      FOR (j = 1; j<8; j ++) {
        code_enh[i] = (unsigned char)(add(code_enh[i], shl(*ptr_enh++, j)));
        move16();
      }
    }
  }

  /*****************************/
#ifdef DYN_RAM_CNT
  DYN_RAM_POP();
#endif
  /*****************************/

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
    work->plcwork = G722PLC_init();

#ifdef DYN_RAM_CNT
    {
      UWord32 ssize = 0;
#ifdef MEM_STT
      ssize += (UWord32)(sizeof(g722dec_state));
#endif
      DYN_RAM_PUSH(ssize, "dummy");
    }
#endif

  }
  return (void *)work;
}

void g722_decode_dest(void *ptr)
{
  g722dec_state *work = (g722dec_state *)ptr;


  if (work != NULL) {
    G722PLC_clear(work->plcwork);
    free(work);
    /*****************************/
#ifdef DYN_RAM_CNT
    DYN_RAM_POP();
#endif
    /*****************************/
  }
  return;
}

void g722_decode_reset(void *ptr)
{
  g722dec_state *work = (g722dec_state *)ptr;

  Word16 * w16ptr;

#ifdef DYN_RAM_CNT
  {
    UWord32 ssize = 0;
    ssize += (UWord32) ((2) * SIZE_Ptr);
    ssize += (UWord32) ((0) * SIZE_Word16);
    ssize += (UWord32) ((0) * SIZE_Word32);

    DYN_RAM_PUSH(ssize, "dummy");
  }
#endif

  if (work != NULL) {
    w16ptr = (Word16*)ptr;
    zero16(sizeof(g722dec_state)/2,w16ptr); /*106 : size of g722dec_state structure in Word16 (212 bytes)*/
    work->g722work.detl = 32; move16();
    work->g722work.deth = 8; move16();

  }
  /*****************************/
#ifdef DYN_RAM_CNT
  DYN_RAM_POP();
#endif
  /*****************************/
  return;
}
/* .................... end of g722_decode_reset() ....................... */
/*
Word32 G722PLC_decode(short *code, short *outcode, short mode, Word16 read1,
g722_state *decoder,void *plc_state)
*/
static Word16 g722_decode1(Word16 mode, const unsigned char *code,
                           const unsigned char *code_enh, Word16 mode_enh, Word16 *rl, Word16 i, 
                           void *ptr, UWord16 **pBit_wbenh, Word16 wbenh_flag, Word16 *enh_no, Word32 *i_sum

                           )
{
  Word16          il, ih;
  Word16          rh, k, nb, ih_enh;

  g722dec_state *work = (g722dec_state *)ptr;            
  g722_state *decoder = &(work->g722work);
  G722PLC_STATE *plc_state = (G722PLC_STATE *)(work->plcwork);

#ifdef DYN_RAM_CNT
  {
    UWord32 ssize = 0;
    ssize += (UWord32) ((3) * SIZE_Ptr);
    ssize += (UWord32) ((6) * SIZE_Word16);
    ssize += (UWord32) ((0) * SIZE_Word32);

    DYN_RAM_PUSH(ssize, "dummy");
  }
#endif
  /* Separate the input G722 codeword: bits 0 to 5 are the lower-band
  * portion of the encoding, and bits 6 and 7 are the upper-band
  * portion of the encoding */
  il = s_and(code[i], 0x3F); /* 6 bits of low SB */
  ih = lshr(code[i], 6); /* 2 bits of high SB */

  ih_enh = 0; move16();
  IF (sub(mode_enh,2) == 0) {                                      /* i 0 1 2 3 4 5 6 7 8 9 a b c d e f */
    k = shr(i, 3);                                               /* k 0 0 0 0 0 0 0 0 1 1 1 1 1 1 1 1 mode2*/
    nb = s_and(i, 7);                                            /*nb 0 1 2 3 4 5 6 7 0 1 2 3 4 5 6 7 mode2*/   
    ih_enh = s_and(shr(code_enh[k],nb), 1);                      
  }                                                                

  /* Call the upper and lower band ADPCM decoders */
  *rl = lsbdec(il, mode, decoder);
  rh = hsbdec_enh(ih, ih_enh, mode_enh, decoder, i, pBit_wbenh, wbenh_flag, enh_no, i_sum);

  /* remove-DC filter */
  rh = G722PLC_hp(&plc_state->mem_hpf_in, &plc_state->mem_hpf_out_hi, &plc_state->mem_hpf_out_lo, rh,
    G722PLC_b_hp156, G722PLC_a_hp156);
  /*****************************/
#ifdef DYN_RAM_CNT
  DYN_RAM_POP();
#endif
  /*****************************/
  return(rh);
}

void g722_decode(Word16 mode, const unsigned char *code,
                 const unsigned char *code_enh, Word16 mode_enh,
                 int loss_flag, Word16 *outcode,
                 void *ptr, UWord16 **pBit_wbenh, Word16 wbenh_flag
                 )
{

  g722dec_state *work = (g722dec_state *)ptr;            
  g722_state *decoder = &(work->g722work);               
  G722PLC_STATE *plc_state = (G722PLC_STATE *)(work->plcwork);


#ifdef DYN_RAM_CNT
  {
    UWord32 ssize = 0;
    ssize += (UWord32) ((3) * SIZE_Ptr);
    ssize += (UWord32) ((0) * SIZE_Word16);
    ssize += (UWord32) ((0) * SIZE_Word32);

    DYN_RAM_PUSH(ssize, "dummy");
  }
#endif
  IF (loss_flag == 0) {
    /*------ decode good frame ------*/

    /* Decoder variables */
    Word16          rl, rh;

    /* Auxiliary variables */
    Word16             i, j;  
    Word16 *ptr_l, *ptr_h;
    Word16  filtmem[L_FRAME_WB+22];
    Word16 *filtptr = &filtmem[L_FRAME_WB];
    Word16 weight;

    Word16 enh_no;
    Word32 i_sum;

#ifdef DYN_RAM_CNT
    {
      UWord32 ssize = 0;
      ssize += (UWord32) ((3) * SIZE_Ptr);
      ssize += (UWord32) ((L_FRAME_WB+22 + 6) * SIZE_Word16);
      ssize += (UWord32) ((1) * SIZE_Word32);


      DYN_RAM_PUSH(ssize, "dummy");
    }
#endif
    /* shift speech buffers */
    mov16(257, &plc_state->mem_speech[L_FRAME_NB], plc_state->mem_speech); /*shift 5 ms*/
    mov16(120, &plc_state->mem_speech_hb[L_FRAME_NB], plc_state->mem_speech_hb); /*shift 5 ms*/

    ptr_l = &(plc_state->mem_speech[257]);   
    ptr_h = &(plc_state->mem_speech_hb[120]);   

    /* Decode - reset is never applied here */
    i = 0; move16();

    mov16(22, &decoder->qmf_rx_delayx[2], &filtmem[L_FRAME_WB]); /* load memory */

    enh_no = NBITS_MODE_R1SM_WBE; move16();

    i_sum = 0; move32();

    IF(sub(plc_state->count_crossfade,CROSSFADELEN) < 0) /* first good 10 ms, crossfade is needed*/
    {
      FOR (i = plc_state->count_crossfade; i < 20; i++) /*first 20 samples : flat part*/
      {
        /* Separate the input G722 codeword: bits 0 to 5 are the lower-band
        * portion of the encoding, and bits 6 and 7 are the upper-band
        * portion of the encoding */
        rh = g722_decode1(mode, code, code_enh, mode_enh, &rl, i, 
          ptr, pBit_wbenh, wbenh_flag, &enh_no, &i_sum);
        /* cross-fade samples with PLC synthesis (in lower band only) */
        rl = mult(plc_state->crossfade_buf[i], 32767);

        /* copy lower and higher band sample */
        *ptr_l++ = rl; move16();
        *ptr_h++ = rh; move16();

        /* Calculation of output samples from QMF filter */
        qmf_rx_buf (rl, rh, &filtptr, &outcode);
      }

      weight = 546; move16(); /*first valid frame, after flat part (sample 21)*/
      if(plc_state->count_crossfade > 0)/* 2nd valid frame*/
      {
        weight = 11466; move16(); /*21*546*/
      }
      FOR (; i < add(plc_state->count_crossfade, L_FRAME_NB); i++)
      {
        j =  sub(i, plc_state->count_crossfade);
        /* Separate the input G722 codeword: bits 0 to 5 are the lower-band
        * portion of the encoding, and bits 6 and 7 are the upper-band
        * portion of the encoding */
        rh = g722_decode1(mode, code, code_enh, mode_enh, &rl, j, 
          ptr, pBit_wbenh, wbenh_flag, &enh_no, &i_sum);
        /* cross-fade samples with PLC synthesis (in lower band only) */
        rl = add(mult(rl, weight), mult(plc_state->crossfade_buf[i], sub(32767, weight)));
        weight = add(weight, 546);

        /* copy lower and higher band sample */
        *ptr_l++ = rl; move16();
        *ptr_h++ = rh; move16();

        /* Calculation of output samples from QMF filter */
        qmf_rx_buf (rl, rh, &filtptr, &outcode);

      }
      plc_state->count_crossfade = add(plc_state->count_crossfade, L_FRAME_NB); move16();
    }

    FOR (; i < L_FRAME_NB; i++)
    {
      /* Separate the input G722 codeword: bits 0 to 5 are the lower-band
      * portion of the encoding, and bits 6 and 7 are the upper-band
      * portion of the encoding */
      rh = g722_decode1(mode, code, code_enh, mode_enh, &rl, i, 
        ptr, pBit_wbenh, wbenh_flag, &enh_no, &i_sum);
      /* copy lower and higher band sample */
      *ptr_l++ = rl; move16();
      *ptr_h++ = rh; move16();

      /* Calculation of output samples from the reference QMF filter */
      qmf_rx_buf (rl, rh, &filtptr, &outcode);
    }

    mov16(22, filtmem, &decoder->qmf_rx_delayx[2]); /* save memory */

    /* set previous bfi to good frame */
    plc_state->prev_bfi = 0; move16();
    /*****************************/
#ifdef DYN_RAM_CNT
    DYN_RAM_POP();
#endif
    /*****************************/
  }
  ELSE { /* (loss_flag != 0) */

    /*------ decode bad frame ------*/
    G722PLC_conceal(plc_state, outcode, decoder);

    /* set previous bfi to good frame */
    plc_state->prev_bfi = 1; move16();
  }



  /*****************************/
#ifdef DYN_RAM_CNT
  DYN_RAM_POP();
#endif
  /*****************************/
  return;
}
/* .................... end of g722_decode() .......................... */
