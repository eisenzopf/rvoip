/* ITU G.722 3rd Edition (2012-09) */

/*--------------------------------------------------------------------------
 ITU-T Annex B (ex G.722-SWB) Source Code
 Software Release 1.00 (2010-09)
 (C) 2010 France Telecom, Huawei Technologies, NTT, VoiceAge Corp.
--------------------------------------------------------------------------*/

#include "bit_op.h"
#include "errexit.h"
#include "pcmswb_common.h"
#include "softbit.h"
#include "qmfilt.h"
#include "g722.h"
#include "bwe.h"
#include "avq.h"

/*****************************/
#ifdef DYN_RAM_CNT
#include "dyn_ram_cnt.h"
#endif
/*****************************/

#define OK  0
#define NG  1

#define moveADDR()      move16()

#ifdef WMOPS
extern Word16           Id;
#endif

typedef struct {
  Word16  Mode;               /* Decoding mode */
  Word16  OpFs;               /* Sampling frequency */
  Word16  gain_ns;            /* Noise shaping gain */
  void*   G722_SubDecoder;    /* Work space for G.722 */
  void*   SubDecoderSH;       /* Work space for super-higher-band sub-decoder */
  void*   SubDecoderBWE;      /* Work space for 8kbps swb extension */
  void*   pQMFBuf_SWB;        /* QMF filter buffer for SWB */
  Word16 prev_Mode;
  Word16 prev2_Mode;
  Word16 prev_ploss_status;
  Word16 sattenu; /* Q(15) */
  Word16 sattenu1; /* Q(15) */
  Word16 sattenu3;/* Q(15) */
  Word16 sprev_fenv[8];
  Word16 prev_bit_switch_flag, bit_switch_count;
  Word16 bit_switch_status;
} pcmswbDecoder_WORK;

/*----------------------------------------------------------------
Function:
PCM SWB decoder constructor
Return value:
Pointer to work space
----------------------------------------------------------------*/
void *pcmswbDecode_const(
                         Word16  mode   /* (i): Decoding mode      */
                         )
{
  pcmswbDecoder_WORK *w=NULL;

  /* Static memory allocation */
  w = (void *)malloc( sizeof(pcmswbDecoder_WORK) );
  if ( w == NULL )  return NULL;

  /*****************************/
#ifdef DYN_RAM_CNT
  {
    UWord32 ssize;
    ssize = (UWord32) (0);
#ifdef MEM_STT
    ssize += (UWord32) (sizeof(pcmswbDecoder_WORK));
#endif
    DYN_RAM_PUSH(ssize, "dummy");
  }
#endif
  /*****************************/
  w->Mode = mode;		move16();
  w->prev_Mode = -1;	move16();
  w->prev2_Mode = -1;	move16();

  SWITCH (w->Mode) {
  case MODE_R00wm : w->OpFs = 16000; move16(); BREAK;
  case MODE_R0wm  : w->OpFs = 16000; move16(); BREAK;
  case MODE_R1wm  : w->OpFs = 16000; move16(); BREAK;
  case MODE_R1sm  : w->OpFs = 32000; move16(); BREAK;
  case MODE_R2sm  : w->OpFs = 32000; move16(); BREAK;
  case MODE_R3sm  : w->OpFs = 32000; move16(); BREAK;
  default : error_exit("Decoding mode error.");
  }

  w->G722_SubDecoder = g722_decode_const();
  w->pQMFBuf_SWB = QMFilt_const(NTAP_QMF_SWB, sSWBQmf0, sSWBQmf1);
  w->SubDecoderBWE = bwe_decode_const();

  /* constructor for AVQ decoder */
  IF (sub(w->Mode, MODE_R1sm) >= 0)
  {
    w->SubDecoderSH = avq_decode_const();
  }

  pcmswbDecode_reset( (void *)w );

  return (void *)w;
}

/*----------------------------------------------------------------
Function:
PCM SWB decoder set mode and output SF in bitrateswitch mode
Return value:
Pointer to work space
----------------------------------------------------------------*/
Word16 pcmswbDecode_set(
                        Word16  mode,   /* (i): Decoding mode      */
                        void*  p_work  /* (i/o): Work space */
                        )
{
  pcmswbDecoder_WORK *w=(pcmswbDecoder_WORK *)p_work;

#ifdef DYN_RAM_CNT
  DYN_RAM_PUSH((UWord32) SIZE_Ptr, "dummy");
#endif
  w->Mode = mode; move16();
  w->OpFs = 32000; move16();

  /*****************************/
#ifdef DYN_RAM_CNT
  DYN_RAM_POP();
#endif
  /*****************************/  
  return OK;
}

/*----------------------------------------------------------------
Function:
PCM SWB decoder destructor
Return value:
None
----------------------------------------------------------------*/
void pcmswbDecode_dest(
                       void*  p_work  /* (i): Work space */
                       )
{
  pcmswbDecoder_WORK *w=(pcmswbDecoder_WORK *)p_work;

  if( w != NULL ) {
    g722_decode_dest( w->G722_SubDecoder );
    QMFilt_dest( w->pQMFBuf_SWB );              /* QMF for SWB */

    bwe_decode_dest( w->SubDecoderBWE );

    /* destructor for AVQ decoder */
    IF (sub(w->Mode, MODE_R1sm) >= 0)
    {
      avq_decode_dest (w->SubDecoderSH);
    }

    free( w );

    /*****************************/
#ifdef DYN_RAM_CNT
    DYN_RAM_POP();
#endif
    /*****************************/  

  }
}

/*----------------------------------------------------------------
Function:
PCM SWB decoder reset
Return value:
OK
----------------------------------------------------------------*/
Word16  pcmswbDecode_reset(
                           void*  p_work  /* (i/o): Work space */
                           )
{
  pcmswbDecoder_WORK *w=(pcmswbDecoder_WORK *)p_work;

#ifdef DYN_RAM_CNT
  DYN_RAM_PUSH((UWord32) SIZE_Ptr, "dummy");
#endif
  if( w != NULL ) {/* use if, this is implementation dependant. Should never happen */
    QMFilt_reset( w->pQMFBuf_SWB );            /* QMF for SWB */

    bwe_decode_reset( w->SubDecoderBWE );

    /* reset for AVQ decoder */
    IF (sub(w->Mode, MODE_R1sm) >= 0)
    {
      avq_decode_reset (w->SubDecoderSH);
    }
    w->gain_ns = 32767; move16();  /* start with full gain */
    w->sattenu = 3277; move16();   /* Q(15) */ 
    w->sattenu1 = 3277; move16();  /* Q(15) */ 
    w->sattenu3 = 32767; move16(); /* Q(15) */ 
    w->prev_ploss_status = 0; move16();
    w->prev_bit_switch_flag = 0; move16();
    w->bit_switch_count = 0; move16();
    w->bit_switch_status = 0; move16();
    zero16( 8, w->sprev_fenv);
  }
  /*****************************/
#ifdef DYN_RAM_CNT
  DYN_RAM_POP();
#endif
  /*****************************/  
  return OK;
}

void bst_frame_G722(unsigned char *bptframe, /*i: layered frame bitstream*/ 
                    unsigned char *bptg722  /*o: g722 style bitstream, scalability by 2 samples, can be the same as the input*/
                    )
{
  /*  write [ b2*n, b3*n, b4*n, b5*n, b6*n, b7*n, b1*n, b0*n]  to enable truncation of G.722 g192 frames */
  Word16 decal;
  Word16 il;
  Word16 j, i;
  unsigned char *bpttmp;
  bpttmp = malloc(L_FRAME_NB);
  /*****************************/
#ifdef DYN_RAM_CNT
  DYN_RAM_PUSH((UWord32) (SIZE_Ptr + 4 * SIZE_Word16 + L_FRAME_NB), "dummy");
#endif
  /*****************************/
  zero16(L_FRAME_NB/2, (Word16 *)bpttmp);
  FOR (j = 0; j < L_FRAME_NB; j++) {

    decal = shr(j,3);
    FOR(i = 2;  i < 8; i++) { /*from b2 to b7*/
      il = shl(s_and(bptframe[decal],0x01), i);  /*  bi in LSB position, shift to position i*/
      bpttmp[j] = (unsigned char)add(bpttmp[j], il); /*compose g722 codeword b7b6b5b4b3b2b1b0*/
      bptframe[decal] = (unsigned char)shr(bptframe[decal], 1); /*shift out read bit, next bit in LSB position*/
      decal = add(decal, 5);
      move16();
      move16();
    }
    il = shl(s_and(bptframe[decal],0x01), 1);  /*  b1 in LSB position, shift to position 1*/
    bpttmp[j] = (unsigned char)add(bpttmp[j], il); /*compose g722 codeword b7b6b5b4b3b2b1b0*/
    bptframe[decal] = (unsigned char)shr(bptframe[decal], 1); /*shift out read bit, next bit in LSB position*/
    decal = add(decal, 5);
    il = s_and(bptframe[decal],0x01);  /*  b1 in LSB position, shift to position 1*/
    bpttmp[j] = (unsigned char)add(bpttmp[j], il); /*compose g722 codeword b7b6b5b4b3b2b1b0*/
    bptframe[decal] = (unsigned char)shr(bptframe[decal], 1); /*shift out read bit, next bit in LSB position*/
    move16();
    move16();
    move16();
    move16();
  }
  mov16(L_FRAME_NB/2, (Word16*)bpttmp, (Word16*)bptg722);
  free(bpttmp);

  /*****************************/
#ifdef DYN_RAM_CNT
  DYN_RAM_POP();
#endif
  /*****************************/
  return;
}

/*----------------------------------------------------------------
Function:
PCM SWB decoder
Return value:
OK/NG
----------------------------------------------------------------*/
Word16  pcmswbDecode(
                     const unsigned char*  bitstream,   /* (i):   Input bitstream  */
                     Word16*                outwave,     /* (o):   Output signal    */
                     void*                 p_work,      /* (i/o): Work space       */
                     Word16                   ploss_status /* (i):   Packet-loss flag */
                     ) 
{
  unsigned char  *bpt = (unsigned char  *)bitstream, *bptmp;
  unsigned char  bst_g722[NBytesPerFrame_G722_64k];
  Word16  SubSigSuperWideLowQMF[L_FRAME_WB];
  Word16  SubSigSuperWideHighQMF[L_FRAME_WB];  /* 8-14 kHz signal (high, 80 points) */
  Word16  i;
  pcmswbDecoder_WORK *w=(pcmswbDecoder_WORK *)p_work;
  UWord16 bst_buff2[NBitsPerFrame_SWB_2], *pBit_SVQ2; /* Memory related in softbit is not counted */
  UWord16 *pBit_SVQ;
  UWord16 *pBit_wbenh, *pBit_BWE, bst_buff[NBitsPerFrame_SWB_1]; /* Memory related in softbit is not counted */
  Word16  mode_enh;
  Word16  wbenh_flag;
  Word16  index_g; /* 5bit index of frame gain */
  Word16  sTenv_SWB[SWB_TENV]; /* Q(0) */
  Word16  scoef_SWB[SWB_F_WIDTH]; /* Q(scoef_SWBQ) */
  Word16  sFenv_SVQ[SWB_NORMAL_FENV]; /* Q(scoef_SWBQ) */
  Word16  scoef_SWBQ;
  Word16  cod_Mode, T_modify_flag;
  Word16  layers_SWB;
  Word16  bit_switch_flag;
  Word16 ploss_status_buff;

  /* G.722 core only */
  Word16 G722mode;
  Word16 nbytesPerFrame;
  Word16 *outsig;

  /* initialize */
  layers_SWB = 0;        move16();
  scoef_SWBQ = 0;        move16();
  bit_switch_flag = 0;   move16();
  ploss_status_buff = 0; move16();
  wbenh_flag = 0;        move16();
  zero16( NBitsPerFrame_SWB_1, (Word16*)bst_buff);
  zero16( NBitsPerFrame_SWB_2, (Word16*)bst_buff2);
  zero16( SWB_TENV, sTenv_SWB);
  zero16( SWB_F_WIDTH, scoef_SWB);
  zero16( SWB_NORMAL_FENV, sFenv_SVQ);

  /*****************************/
#ifdef DYN_RAM_CNT
  {
    UWord32 ssize;
    ssize = (UWord32) (8 * SIZE_Ptr);
    ssize += (UWord32) ((2 * L_FRAME_NB + NBitsPerFrame_SWB_1 + NBitsPerFrame_SWB_2 + SWB_TENV + SWB_F_WIDTH + SWB_NORMAL_FENV + 12) * SIZE_Word16);
    ssize += (UWord32) (NBytesPerFrame_G722_64k);		/* unsigned char */
    DYN_RAM_PUSH(ssize, "dummy");
  }
#endif
  /*****************************/

  if (p_work == NULL)
  {
    /*****************************/
#ifdef DYN_RAM_CNT
    DYN_RAM_POP();
#endif
    /*****************************/
    return NG;
  }

  zero16(L_FRAME_WB,SubSigSuperWideHighQMF);

  /* ------------------------------------------- */
  /* Super higher-band enhancement layer decoder */
  /* ------------------------------------------- */
  test();
  IF (sub(w->Mode, MODE_R1sm) == 0) { /* G.722 */
    bptmp = bpt + NBytesPerFrame_G722_56k;	
    hardbit2softbit( NBytesPerFrame_R1SM, bptmp, bst_buff );
  }
  ELSE IF (sub(w->Mode, MODE_R2sm) >= 0)
  {
    bptmp = bpt + NBytesPerFrame_G722_64k;	
    hardbit2softbit( NBytesPerFrame_SWB_1, bptmp, bst_buff );
    IF (sub(w->Mode, MODE_R3sm) == 0) 
    {
      bptmp += NBytesPerFrame_SWB_1 + NBytesPerFrame_SWB_2/2;	
      hardbit2softbit( NBytesPerFrame_SWB_2/2, bptmp, bst_buff2 );
    }
  }

  test();
  IF (sub(w->Mode, MODE_R00wm) == 0) {
    G722mode = 3;
    nbytesPerFrame = NBytesPerFrame_G722_48k;
    move16();
    move16();
  }
  ELSE IF ((sub(w->Mode, MODE_R0wm) == 0) ||
    (sub(w->Mode, MODE_R1sm) == 0)) {
      G722mode = 2;
      nbytesPerFrame = NBytesPerFrame_G722_56k;
      move16();
      move16();
  }
  ELSE { /* MODE_R1wm || MODE_R2sm || MODE_R3sm */
    G722mode = 1;
    nbytesPerFrame = NBytesPerFrame_G722_64k;
    move16();
    move16();
  }
  IF (sub(w->OpFs,16000 ) == 0) {
    outsig = outwave;
  }
  ELSE { /* w->OpFs == 32000 */
    outsig = SubSigSuperWideLowQMF;
  }

  /* G.722 decoding */
  IF (sub(w->Mode, MODE_R1sm) >= 0)
  {
    test();
    IF ((sub(bst_buff[0], G192_BITONE) == 0) && (sub(bst_buff[1], G192_BITONE) == 0))
    {    
      wbenh_flag = 0;
      move16();
    }
    ELSE
    { 
      wbenh_flag = 1;
      pBit_wbenh = &bst_buff[NBITS_MODE_R1SM_BWE];
      move16();
    } 
  }
  mode_enh=3;
  move16();
  if (sub(w->Mode, MODE_R3sm) == 0) {

    mode_enh=G722EL1_MODE; /*mode_enh = 2 if 1 bit/sample for BWE*/
    move16();
  }
  bptmp = bpt + (NBytesPerFrame_G722_64k+NBytesPerFrame_SWB_1); /*used only in MODE_R3sm */ 
  FOR (i=0; i<nbytesPerFrame; i++) 
  {
    bst_g722[i] = bpt[i];
    move16();
  }
  FOR (i=nbytesPerFrame; i<NBytesPerFrame_G722_64k; i++) 
  {
    bst_g722[i] = 0;
    move16();
  }
  bst_frame_G722(bst_g722, bst_g722);

  g722_decode(G722mode, bst_g722, bptmp, mode_enh, ploss_status, outsig, w->G722_SubDecoder,
    &pBit_wbenh, wbenh_flag);
  bpt += nbytesPerFrame;	

  /* --------------------- */
  /* Band reconstructing   */
  /* --------------------- */
  test();  
  IF ( (sub( w->Mode, MODE_R1sm ) >= 0)  || (sub( w->prev_Mode, MODE_R1sm ) >= 0) )
  {
    test(); test();
    IF(w->prev_Mode < 0)
    {
      w->prev2_Mode = w->Mode;
      w->bit_switch_count = 0; move16();
    }
    ELSE IF((sub( w->Mode, MODE_R1sm ) < 0) && (sub( w->prev_Mode, MODE_R1sm ) >= 0))
    {
      w->prev2_Mode = w->Mode; move16();
      bit_switch_flag = 1; move16();
      w->bit_switch_count ++; move16();
    }
    ELSE IF((sub( w->Mode, MODE_R1sm ) >= 0) && (sub( w->prev2_Mode, MODE_R1sm ) < 0))
    {
      bit_switch_flag = 2; move16();
      w->bit_switch_count = 0; move16();
    }
    ELSE
    {
      w->bit_switch_count = 0; move16();
    }
    pBit_BWE = bst_buff;

    cod_Mode = NORMAL; move16();
    IF(sub( w->Mode, MODE_R1sm ) >= 0)
    {
      UWord16 *pBit = pBit_BWE;
      cod_Mode = GetBit(&pBit, 2);
    }
    ploss_status_buff = ploss_status; move16();
    test();
    if (sub(w->prev_ploss_status, 1)==0 && sub(cod_Mode, 1)<=0 && sub (w->Mode, MODE_R1sm) != 0)
    {
      ploss_status = 1; move16();
    }

    T_modify_flag =
      bwe_dec_freqcoef( &pBit_BWE, SubSigSuperWideLowQMF, w->SubDecoderBWE, 
      &cod_Mode, 
      sTenv_SWB,     /* Q(0) */
      scoef_SWB,     /* Q(scoef_SWBQ) */
      &index_g, 
      sFenv_SVQ,     /* Q(scoef_SWBQ) */
      ploss_status,
      bit_switch_flag,
      w->prev_bit_switch_flag,
      &scoef_SWBQ
      );

    /* Replace MDCT coefficents with those from SVQ in R2SM */
    IF (sub(w->Mode, MODE_R2sm) >= 0)
    {  
      test();
      IF (ploss_status == 0 && w->bit_switch_status == 0)
      {
        layers_SWB = 1;
        move16();
        IF(sub(w->Mode, MODE_R3sm) == 0)
        {
          layers_SWB = 2;
          move16();
        }
        pBit_SVQ = bst_buff + NBITS_MODE_R1SM_TOTLE;
        pBit_SVQ2 = bst_buff2;

        swbl1_decode_AVQ( (void*)w->SubDecoderSH, pBit_SVQ, pBit_SVQ2, (const Word16*)sFenv_SVQ, scoef_SWB, index_g, cod_Mode, layers_SWB, &scoef_SWBQ );
        w->prev_ploss_status = 0; move16();
      }
      ELSE
      {
        bwe_avq_buf_reset(w->SubDecoderSH);

        w->prev_ploss_status = 1; move16();
        IF (ploss_status_buff == 0)
        {
          AVQ_state_dec *wtmp = w->SubDecoderSH;

          w->prev_ploss_status = 0; move16();
          wtmp->pre_cod_Mode = cod_Mode; move16();
        }
      }
    }

    IF(sub(bit_switch_flag, 1) == 0)
    {
      array_oper(60, w->sattenu3, scoef_SWB, scoef_SWB, &mult);
      if(sub((Word16) w->bit_switch_count, 200) > 0)
      {			  
        w->sattenu3 = sub(w->sattenu3, 328); /* Q15 */
      }
      w->sattenu3 = s_max(w->sattenu3, 0);
    }
    ELSE
    {
      w->sattenu3 = 32767; move16();
    }

    IF(sub(bit_switch_flag, 2) == 0)
    {
      array_oper(60, w->sattenu, scoef_SWB, scoef_SWB, &mult);
      w->sattenu = add( w->sattenu, 655); /* Q15 */
      w->prev2_Mode = MODE_R0wm; move16();
      IF(sub( w->sattenu, 32767) == 0) /* if sattenu > 32767, sattenu was saturated to 32767 at last add(). */
      {
        w->sattenu = 3277; move16();
        bit_switch_flag = 0; move16();
        w->prev2_Mode = w->Mode; move16();
      }
    }

    IF(bit_switch_flag == 0) 
    {
      IF(sub((Word16) w->prev_bit_switch_flag, 1) == 0)
      {
        w->sattenu1 = 3277; move16();
      }
    }
    ELSE
    {
      IF(sub( w->sattenu1, 32767) < 0 )
      {
        array_oper(60, w->sattenu1, scoef_SWB, scoef_SWB, &mult);
        w->sattenu1 = add( w->sattenu1, 655);
      }
    }

    /* BWE-based post-processing */
    bwe_dec_timepos( cod_Mode, sTenv_SWB, scoef_SWB, SubSigSuperWideHighQMF, 
      w->SubDecoderBWE, ploss_status, T_modify_flag, &scoef_SWBQ );
  }
  ELSE
  {
    bwe_dec_update( 
      SubSigSuperWideLowQMF,    	   /* (i): Input lower-band WB signal */
      w->SubDecoderBWE             /* (i/o): Pointer to work space        */
      );
  }
  QMFilt_syn(L_FRAME_WB, SubSigSuperWideLowQMF, SubSigSuperWideHighQMF, outwave, w->pQMFBuf_SWB );

  w->bit_switch_status = 0; move16();
  if (sub(w->Mode, MODE_R1sm) < 0)
  {
    w->bit_switch_status = 1; move16();    
  }

  if(sub( bit_switch_flag, 1 ) == 0)
  {
    w->Mode = MODE_R1sm; move16();
  }

  w->prev_Mode = w->Mode; move16();
  w->prev_bit_switch_flag = bit_switch_flag; move16();

  mov16(8, sFenv_SVQ, w->sprev_fenv);

  /*****************************/
#ifdef DYN_RAM_CNT
  DYN_RAM_POP();
#endif
  /*****************************/

  return OK;
}
