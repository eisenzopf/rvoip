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

#include "errexit.h"
#include "pcmswb_common.h"
#include "softbit.h"
#include "prehpf.h"
#include "qmfilt.h"
#include "g722.h"
#include "bwe.h"
#include "avq.h"
#ifdef LAYER_STEREO
#include "g722_stereo.h"
#include "stereo_tools.h"
#endif

/*****************************/
#ifdef DYN_RAM_CNT
#include "dyn_ram_cnt.h"
#endif
/*****************************/

#define OK  0
#define NG  1

/* High-pass filter cutoff definition */
#define FILT_NO_8KHZ_INPUT   5
#define FILT_NO_16KHZ_INPUT  6
#define FILT_NO_32KHZ_INPUT  7

#define moveADDR()      move16()

#ifdef WMOPS
extern Word16 Id;
extern Word16 Id_dmx;
extern short Id_st_enc_swb;
extern short Id_dmx_swb;
#endif

typedef struct {
  Word16  Mode;               /* Encoding mode */
  Word16  OpFs;               /* Sampling frequency */
  Word16  DCBuf[QMF_DELAY_G722];
  void*   pHpassFiltBuf;      /* High-pass filter buffer */
  void*   G722_SubEncoder;    /* Work space for G.722 */
  void*   SubEncoderSH;       /* Work space for super-higher-band sub-encoder */
  void*   SubEncoderBWE;      /* Work space for 8kbps swb extension to G.722 */
  void*   pQmfBuf_SWB;        /* QMF buffer for SWB input */
#ifdef LAYER_STEREO
  Word16  frame_idx;
  Word16  framelen;
  Word16  channel;
  void*   G722_stereo_SubEncoder;
  void*   pQmfBuf_SWB_left;
  void*   pQmfBuf_SWB_right;
  void*   pHpassFiltBuf_L;      /* High-pass filter buffer for L channel */
  void*   pHpassFiltBuf_R;      /* High-pass filter buffer for R channel */
#endif
} pcmswbEncoder_WORK;

/*----------------------------------------------------------------
Function:
PCM SWB encoder constructor
Return value:
Pointer to work space
----------------------------------------------------------------*/
void *pcmswbEncode_const(
                         UWord16 sampf, /* (i): Input sampling rate (Hz) */
                         Word16 mode              /* (i): Encoding mode            */
#ifdef LAYER_STEREO
                        ,Word16 channel
#endif
                         )
{
  pcmswbEncoder_WORK *w=NULL;

  /* Static memory allocation */
  w = (void *)malloc(sizeof(pcmswbEncoder_WORK));
  if (w == NULL)  return NULL;

  /*****************************/
#ifdef DYN_RAM_CNT
  {
    UWord32 ssize;
    ssize = (UWord32) (0);
#ifdef MEM_STT
    ssize += (UWord32) (sizeof(pcmswbEncoder_WORK));
#endif
    DYN_RAM_PUSH(ssize, "dummy");
  }
#endif
  /*****************************/
  w->Mode = mode;       move16();
  w->OpFs = 32000;      move16();   /* Input sampling rate is 32kHz in default */
#ifdef LAYER_STEREO
  w->channel = channel; move16();
#endif
  if (sub(sampf, 16000) == 0)
  {
    w->OpFs = 16000;   move16();  /* Input sampling rate is 16 kHz */
  }
#ifdef LAYER_STEREO
  w->framelen = L_FRAME_SWB; move16();
  if (sub(sampf, 8000) == 0)
  {
     w->framelen = L_FRAME_NB; move16();
  }
  if (sub(sampf, 16000) == 0)
  {
     w->framelen = L_FRAME_WB; move16();
  }
#endif
  zero16( QMF_DELAY_G722, w->DCBuf );
  test(); 
#ifdef LAYER_STEREO
  IF ( (w->Mode < 0) || (sub(w->Mode, 11) >0) ) {
#else
  IF ( (w->Mode < 0) || (sub(w->Mode, 5) >0) ) {
#endif
    error_exit( "Encoding mode error." );
  }

#ifdef LAYER_STEREO
  w->frame_idx = 0;

  w->pHpassFiltBuf_L = highpass_1tap_iir_const();
  if ( w->pHpassFiltBuf_L == NULL )  error_exit( "HPF init error." );
  w->pHpassFiltBuf_R = highpass_1tap_iir_const();
  if ( w->pHpassFiltBuf_R == NULL )  error_exit( "HPF init error." );

  w->pQmfBuf_SWB_left  = QMFilt_const(NTAP_QMF_SWB, sSWBQmf0, sSWBQmf1);
  w->pQmfBuf_SWB_right = QMFilt_const(NTAP_QMF_SWB, sSWBQmf0, sSWBQmf1);
  if ( w->pQmfBuf_SWB_left == NULL || w->pQmfBuf_SWB_right == NULL )  error_exit( "STERO SWB QMF init error." );

  w->G722_stereo_SubEncoder = g722_stereo_encode_const();
#endif
  w->pHpassFiltBuf = highpass_1tap_iir_const();
  w->pQmfBuf_SWB = QMFilt_const(NTAP_QMF_SWB, sSWBQmf0, sSWBQmf1);
  w->G722_SubEncoder = g722_encode_const();
  w->SubEncoderBWE = bwe_encode_const();

  /* constructor for AVQ encoder */
  test(); test();
#ifdef LAYER_STEREO
  test(); test(); test(); test();
  IF (w->Mode == MODE_R1sm  || w->Mode == MODE_R2sm || w->Mode == MODE_R3sm|| w->Mode == MODE_R2ss|| w->Mode == MODE_R3ss|| w->Mode == MODE_R4ss|| w->Mode == MODE_R5ss)
#else
  IF (w->Mode == MODE_R1sm  || w->Mode == MODE_R2sm || w->Mode == MODE_R3sm)
#endif
  {
    w->SubEncoderSH = avq_encode_const();
  }

  pcmswbEncode_reset( (void *)w );

  return (void *)w;
}

/*----------------------------------------------------------------
Function:
PCM SWB encoder destructor
Return value:
None
----------------------------------------------------------------*/
void pcmswbEncode_dest(
                       void*  p_work   /* (i): Work space */
                       )
{
  pcmswbEncoder_WORK *w=(pcmswbEncoder_WORK *)p_work;

  if( w != NULL ) {
    highpass_1tap_iir_dest( w->pHpassFiltBuf );
    QMFilt_dest( w->pQmfBuf_SWB );                /* QMF for SWB */
#ifdef LAYER_STEREO
    highpass_1tap_iir_dest( w->pHpassFiltBuf_L );
    highpass_1tap_iir_dest( w->pHpassFiltBuf_R );
    g722_stereo_encode_dest(w->G722_stereo_SubEncoder);
    QMFilt_dest( w->pQmfBuf_SWB_left );
    QMFilt_dest( w->pQmfBuf_SWB_right );
#endif
    g722_encode_dest(w->G722_SubEncoder);
    bwe_encode_dest( w->SubEncoderBWE );

    /* destructor for AVQ encoder */
    test(); test();
#ifdef LAYER_STEREO
    test(); test(); test(); test();
    IF (w->Mode == MODE_R1sm  || w->Mode == MODE_R2sm || w->Mode == MODE_R3sm|| w->Mode == MODE_R2ss|| w->Mode == MODE_R3ss|| w->Mode == MODE_R4ss|| w->Mode == MODE_R5ss)
#else
    IF (w->Mode == MODE_R1sm  || w->Mode == MODE_R2sm || w->Mode == MODE_R3sm)
#endif
    {
      avq_encode_dest (w->SubEncoderSH);
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
PCM SWB encoder reset
Return value:
OK
----------------------------------------------------------------*/
Word16  pcmswbEncode_reset(
                           void*  p_work   /* (i/o): Work space */
                           )
{
  pcmswbEncoder_WORK *w=(pcmswbEncoder_WORK *)p_work;
#ifdef DYN_RAM_CNT
  DYN_RAM_PUSH((UWord32) SIZE_Ptr, "dummy");
#endif

  if( w != NULL )
  {
    highpass_1tap_iir_reset(w->pHpassFiltBuf);
    QMFilt_reset( w->pQmfBuf_SWB );             /* QMF for SWB */
    g722_encode_reset(w->G722_SubEncoder);
    bwe_encode_reset( w->SubEncoderBWE );
#ifdef LAYER_STEREO
    highpass_1tap_iir_reset(w->pHpassFiltBuf_L);
    highpass_1tap_iir_reset(w->pHpassFiltBuf_R);
    g722_stereo_encode_reset(w->G722_stereo_SubEncoder);
#endif
    /* reset for AVQ encoder */
    test(); test();
#ifdef LAYER_STEREO
    test(); test(); test(); test();
    IF (w->Mode == MODE_R1sm  || w->Mode == MODE_R2sm || w->Mode == MODE_R3sm|| w->Mode == MODE_R2ss|| w->Mode == MODE_R3ss|| w->Mode == MODE_R4ss|| w->Mode == MODE_R5ss)
#else
    IF (w->Mode == MODE_R1sm  || w->Mode == MODE_R2sm || w->Mode == MODE_R3sm)
#endif
    {
      avq_encode_reset (w->SubEncoderSH);
    }
  }

  /*****************************/
#ifdef DYN_RAM_CNT
  DYN_RAM_POP();
#endif
  /*****************************/  
  return OK;
}

void bst_G722_frame(unsigned char *bptg722,  /*i: g722 style bitstream, scalability by 2 samples*/
                    unsigned char *bptframe /*o: layered frame bitstream, can be the same as the input*/
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
  {
    UWord32   ssize;
    ssize = SIZE_Ptr + 4 * SIZE_Word16;
#ifdef MEM_STT
    ssize += L_FRAME_NB;
#endif
    DYN_RAM_PUSH(ssize, "dummy");
  }
#endif
  /*****************************/

  zero16(L_FRAME_NB/2, (Word16*)bpttmp);

  FOR (j = 0; j < L_FRAME_NB; j++) {
    decal = shr(j,3);
    FOR(i = 2;  i < 8; i++) { /*from b2 to b7*/
      il = shl(s_and(bptg722[j], shl(0x01, i)), sub(7,i));  /* i = 2 : b2, i = 7 : b7; left aligned*/
      bpttmp[decal] = (unsigned char)add(shr(bpttmp[decal],1),il); /*shift right + add il to MSB position*/
      move16();
      decal = add(decal, 5);
    }
    il = shl(s_and(bptg722[j], 0x02), 6);  /* i = 1 : b1,  left aligned*/
    bpttmp[decal] = (unsigned char)add(shr(bpttmp[decal],1),il); 
    move16();
    decal = add(decal, 5);
    il = shl(s_and(bptg722[j], 0x01), 7);  /* i = 0 : b0,  left aligned*/
    bpttmp[decal] = (unsigned char)add(shr(bpttmp[decal],1),il); 
    move16();
  }
  mov16(L_FRAME_NB/2, (Word16 *)bpttmp, (Word16 *)bptframe);

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
PCM SWB encoder
Return value:
OK/NG
----------------------------------------------------------------*/
Word16  pcmswbEncode(
                     const Word16*    inwave,
                     unsigned char*  bitstream,
                     void*           p_work
                     ) 
{
  Word16  SubSigSuperWideLow[L_FRAME_WB];
  Word16  SubSigSuperWideHigh[L_FRAME_WB];
  unsigned char  *bpt = bitstream;
  unsigned char   bpt_enh[L_FRAME_WB/8]; /* 2 bits/sample at 4 kHz */  
  Word16          mode_enh=3;
  Word16 i;
  Word16 nbbytes_g722[4] = {-1, NBytesPerFrame_G722_64k, NBytesPerFrame_G722_56k, NBytesPerFrame_G722_48k};
  Word16  SubSigSuperWideHigh_temp[QMF_DELAY_G722]; /*G.722 specific*/
  Word16  *SigInQMF;

  pcmswbEncoder_WORK *w=(pcmswbEncoder_WORK *)p_work;

  Word16 transi, wbenh_flag;
  BWE_state_enc *enc_st = (BWE_state_enc *)w->SubEncoderBWE;
  UWord16 bst_buff[NBitsPerFrame_SWB_1], *pBit_wbenh, *pBit_BWE; /* Memory related in softbit is not counted */
  UWord16 *pBit_SVQ;
  Word16 index_g, cod_Mode, T_modify_flag = 0;
  UWord16 bst_buff2[NBitsPerFrame_SWB_2], *pBit_SVQ2; /* Memory related in softbit is not counted */
  Word16 layers_SWB; 
  Word16 sFenv_SWB_unq[SWB_NORMAL_FENV]; /* Q(12) */
  Word16 stEnv[SWB_TENV]; /* Q(0) */
  Word16 scoef_SWB[SWB_F_WIDTH]; /* Q(scoef_SWBQ) */ 
  Word16 sFenv_SWB[SWB_NORMAL_FENV]; /* Q(scoef_SWBQ) */  
  Word16 scoef_SWBQ = 0;  

  /* G.722 core only */
  Word16 G722mode, localmode;
  Word16 nbytesPerFrame;
  Word16 *insig;
#ifdef LAYER_STEREO
  Word16 input_left_s[L_FRAME_SWB];     
  Word16 input_right_s[L_FRAME_SWB];
  Word16 mono[L_FRAME_WB];     
  Word16 mono_swb_s[L_FRAME_WB],side_swb_s[L_FRAME_WB];
  Word16 gain[2];
  /* high pass filtering */
  Word16 input_left_hpf[L_FRAME_SWB];
  Word16 input_right_hpf[L_FRAME_SWB];
  /* QMF signals */
  Word16 input_left_qmf_wb_s[L_FRAME_WB];
  Word16 input_left_qmf_swb_s[L_FRAME_WB];
  Word16 input_right_qmf_wb_s[L_FRAME_WB];
  Word16 input_right_qmf_swb_s[L_FRAME_WB];

  Word16 bpt_stereo_swb[160]; 
#endif
  
  /*****************************/
#ifdef DYN_RAM_CNT
  {
    UWord32 ssize;
    ssize = (UWord32) ((13 + 2 * L_FRAME_WB + QMF_DELAY_G722 + NBitsPerFrame_SWB_1 + NBitsPerFrame_SWB_2
      + 2 * SWB_NORMAL_FENV + SWB_TENV + SWB_F_WIDTH) * SIZE_Word16);
    ssize += (UWord32) (L_FRAME_WB/8);
    ssize += (UWord32) (8 * SIZE_Ptr);
    DYN_RAM_PUSH(ssize, "dummy");
  }
#endif
  /*****************************/

  move16();move16();move16();   
  move16();move16();move16();move16();   

  zero16( NBitsPerFrame_SWB_1, (Word16*)bst_buff);
  zero16( NBitsPerFrame_SWB_2, (Word16*)bst_buff2);
  zero16( SWB_NORMAL_FENV, sFenv_SWB_unq);
  zero16( SWB_TENV, stEnv);
  zero16( SWB_F_WIDTH, scoef_SWB);
  zero16( SWB_NORMAL_FENV, sFenv_SWB);

#ifdef LAYER_STEREO
  IF(sub(w->channel, 2) == 0)
  {
    /* Stereo channel split */
    deinterleave( inwave, input_left_s, input_right_s, w->framelen*2 );
    IF(sub(w->OpFs, 32000) == 0)
    {
      write_index1(bpt_stereo_swb, 1);
     
      /* high-pass filtering L and R channels before processing downmix */ 
      highpass_1tap_iir_stereo( FILT_NO_32KHZ_INPUT, L_FRAME_SWB, (Word16 *)input_left_s,  input_left_hpf,  w->pHpassFiltBuf_L );
      highpass_1tap_iir_stereo( FILT_NO_32KHZ_INPUT, L_FRAME_SWB, (Word16 *)input_right_s, input_right_hpf, w->pHpassFiltBuf_R );
      /* Band splitting with QMF (SWB) */
      QMFilt_ana(L_FRAME_SWB, input_left_hpf,  input_left_qmf_wb_s,  input_left_qmf_swb_s,  w->pQmfBuf_SWB_left );
      QMFilt_ana(L_FRAME_SWB, input_right_hpf, input_right_qmf_wb_s, input_right_qmf_swb_s, w->pQmfBuf_SWB_right );
#ifdef WMOPS_IDX
      setCounter(Id_dmx_swb); 
#endif         
      downmix_swb(input_left_qmf_swb_s, input_right_qmf_swb_s, mono_swb_s,side_swb_s,w->G722_stereo_SubEncoder);
#ifdef WMOPS_IDX
      setCounter(Id_st_enc_swb); 
#endif
      G722_stereo_encoder_shb(mono_swb_s,side_swb_s,w->G722_stereo_SubEncoder,&bpt_stereo_swb[1], w->Mode, gain); 
#ifdef WMOPS_IDX
      setCounter(Id); 
#endif
      downmix(input_left_qmf_wb_s, input_right_qmf_wb_s, mono, w->G722_stereo_SubEncoder,&bpt_stereo_swb[1],w->Mode,&w->frame_idx,w->OpFs);
#ifdef WMOPS_IDX
      setCounter(Id); 
#endif
    }
    ELSE
    {
      write_index1(bpt_stereo_swb, 0);

      /* high-pass filtering L and R channels before processing downmix */ 
      highpass_1tap_iir_stereo(FILT_NO_16KHZ_INPUT, L_FRAME_WB, (Word16 *)input_left_s, input_left_hpf, w->pHpassFiltBuf_L );
      highpass_1tap_iir_stereo(FILT_NO_16KHZ_INPUT, L_FRAME_WB, (Word16 *)input_right_s, input_right_hpf, w->pHpassFiltBuf_R );

      downmix(input_left_hpf, input_right_hpf, mono, w->G722_stereo_SubEncoder,&bpt_stereo_swb[1],w->Mode,&w->frame_idx,w->OpFs);
#ifdef WMOPS_IDX
      setCounter(Id); 
#endif       
    }
  }

  IF(sub(w->channel, 2) == 0)
  {
    SigInQMF = (Word16 *)mono;
  }
  ELSE
  {
#endif
  SigInQMF = (Word16 *)inwave;
#ifdef LAYER_STEREO
  }
#endif

  if (p_work == NULL)
  {
    /*****************************/
#ifdef DYN_RAM_CNT
    DYN_RAM_POP();
#endif
    /*****************************/
    return NG;
  }

  /* ------------------------------- */
  /* Pre-processing & band splitting */
  /* ------------------------------- */
#ifdef LAYER_STEREO
  IF(sub(w->channel, 1) == 0)
  {
#endif
  IF ( sub(w->OpFs, 16000) == 0 ) { /* Wideband input */

    /* High-pass filtering */
    highpass_1tap_iir( FILT_NO_16KHZ_INPUT,
      L_FRAME_WB, (Word16 *)inwave,
      SigInQMF, w->pHpassFiltBuf );
  }
  ELSE { /* w->OpFs == 32000 */  /* Super wideband input */
    /* High-pass filtering */
    highpass_1tap_iir( FILT_NO_32KHZ_INPUT,
      L_FRAME_SWB, (Word16 *)inwave,
      SigInQMF, w->pHpassFiltBuf );
    /* Band splitting with QMF (SWB) */
    QMFilt_ana( L_FRAME_SWB, SigInQMF, SubSigSuperWideLow, SubSigSuperWideHigh, w->pQmfBuf_SWB );
  }
#ifdef LAYER_STEREO
  }
  test(); test(); test();
  IF ((sub(w->Mode, MODE_R2ss) == 0) ||(sub(w->Mode, MODE_R3ss) == 0) || (sub(w->Mode, MODE_R4ss) == 0)|| (sub(w->Mode, MODE_R5ss) == 0))
  {
    mov16(L_FRAME_WB, mono_swb_s, SubSigSuperWideHigh);     
  }
  IF(sub(w->channel, 1) == 0)
  {
#endif
  mov16( QMF_DELAY_G722, &SubSigSuperWideHigh[L_FRAME_WB-QMF_DELAY_G722], SubSigSuperWideHigh_temp );
  mov16_bwd(L_FRAME_WB-QMF_DELAY_G722, SubSigSuperWideHigh+L_FRAME_WB-1-QMF_DELAY_G722, SubSigSuperWideHigh+L_FRAME_WB-1);
  mov16(QMF_DELAY_G722, w->DCBuf, SubSigSuperWideHigh);
  mov16(QMF_DELAY_G722, SubSigSuperWideHigh_temp, w->DCBuf);
#ifdef LAYER_STEREO
  }
  test(); test();
#endif
  test();
  IF (sub(w->Mode, MODE_R00wm) == 0) {
    G722mode = 3;
    nbytesPerFrame = NBytesPerFrame_G722_48k;
    move16();
    move16();
  }
#ifdef LAYER_STEREO
  ELSE IF ((sub(w->Mode, MODE_R0wm) == 0) ||
           (sub(w->Mode, MODE_R1sm) == 0) || 
           (sub(w->Mode, MODE_R1ws) == 0) ||
           (sub(w->Mode, MODE_R2ss) == 0)){
#else    
  ELSE IF ((sub(w->Mode, MODE_R0wm) == 0) ||
    (sub(w->Mode, MODE_R1sm) == 0)) {
#endif
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
#ifdef LAYER_STEREO
  IF(sub(w->channel, 1) == 0)
  {
#endif
  IF (sub(w->OpFs, 16000) == 0) {
    insig = SigInQMF;
  }
  ELSE { /* w->OpFs == 32000 */
    insig = SubSigSuperWideLow;
  }
#ifdef LAYER_STEREO
  }
  ELSE
  {
    insig = mono;
  }
#endif
  localmode = G722mode; 
  move16();
  if(sub(G722mode, 1) == 0)
  {
    localmode = 2; /* scalable noise shaping, 1st stage at 56 kbps */
    move16();
  }
  /* G.722 encoding */
  wbenh_flag = 0;
  move16();

  test(); test();
#ifdef LAYER_STEREO
  test(); test(); test(); test();
  IF ((sub(w->Mode, MODE_R1sm) == 0)|| (sub(w->Mode, MODE_R2sm) == 0) || 
      (sub(w->Mode, MODE_R3sm) == 0)|| (sub(w->Mode, MODE_R2ss) == 0) || 
      (sub(w->Mode, MODE_R3ss) == 0)|| (sub(w->Mode, MODE_R4ss) == 0) || 
      (sub(w->Mode, MODE_R5ss) == 0))
#else
  IF ((sub(w->Mode, MODE_R1sm) == 0) || (sub(w->Mode, MODE_R2sm) == 0) || (sub(w->Mode, MODE_R3sm) == 0))
#endif
  {
    T_modify_flag = Icalc_tEnv( SubSigSuperWideHigh, stEnv,
      &transi, enc_st->preMode
      , (void*)enc_st
      );

    test();
    IF ((sub((Word16) transi, 1) != 0) && (sub((Word16) enc_st->preMode, TRANSIENT) != 0))
    {
      wbenh_flag = 1;
      pBit_wbenh = &bst_buff[NBITS_MODE_R1SM_BWE];
      move16();
    }
  }
#ifdef LAYER_STEREO
  test(); test();
  IF ((sub(w->Mode, MODE_R3sm) == 0)||(sub(w->Mode, MODE_R4ss) == 0)||(sub(w->Mode, MODE_R5ss) == 0)) {
#else
  if (sub(w->Mode, MODE_R3sm) == 0) {
#endif
    mode_enh=G722EL1_MODE; /*mode_enh = 2 if 1 bit/sample for WBE*/
    move16();
  }
  g722_encode(G722mode, localmode, insig, bpt, bpt_enh, mode_enh, w->G722_SubEncoder,
    wbenh_flag, &pBit_wbenh);

  bst_G722_frame(bpt, bpt);
  bpt += nbbytes_g722[G722mode];
  moveADDR();

  /* ------------------------------------------- */
  /* Super-higher-band enhancement layer encoder */
  /* ------------------------------------------- */
  test();
#ifdef LAYER_STEREO
  test(); test(); test(); test();
  IF (sub(w->Mode, MODE_R1sm) == 0 || sub(w->Mode, MODE_R2ss) == 0) { /* G.722 */
#else
  IF (sub(w->Mode, MODE_R1sm) == 0) { /* G.722 */
#endif
    pBit_BWE = bst_buff;
    /* swb encoding */
    bwe_enc( SubSigSuperWideHigh, &pBit_BWE, w->SubEncoderBWE, stEnv, transi,
      &cod_Mode, sFenv_SWB, scoef_SWB, &index_g, T_modify_flag, sFenv_SWB_unq,
      &scoef_SWBQ
#ifdef LAYER_STEREO
     , gain, w->channel
#endif
     );

    softbit2hardbit (NBytesPerFrame_R1SM, bst_buff, bpt);
    bpt += NBytesPerFrame_SWB_0;   
  }
#ifdef LAYER_STEREO
  ELSE IF ((sub(w->Mode, MODE_R2sm) == 0) ||
           (sub(w->Mode, MODE_R3sm) == 0) ||
           (sub(w->Mode, MODE_R3ss) == 0)||
           (sub(w->Mode, MODE_R4ss) == 0)||
           (sub(w->Mode, MODE_R5ss) == 0))
#else
  ELSE IF ((sub(w->Mode, MODE_R2sm) == 0) ||
    (sub(w->Mode, MODE_R3sm) == 0) )
#endif
  {
    pBit_BWE = bst_buff;

    /* swb encoding */
    bwe_enc( SubSigSuperWideHigh, &pBit_BWE, w->SubEncoderBWE, stEnv, transi,
      &cod_Mode, sFenv_SWB, scoef_SWB, &index_g, T_modify_flag, sFenv_SWB_unq,
      &scoef_SWBQ
#ifdef LAYER_STEREO
     , gain, w->channel
#endif     
     );

    layers_SWB = 1;   move16();
#ifdef LAYER_STEREO
    test(); test();
    IF( sub(w->Mode, MODE_R3sm) == 0 || sub(w->Mode, MODE_R4ss) == 0 || sub(w->Mode, MODE_R5ss) == 0)
#else
    IF( sub(w->Mode, MODE_R3sm) == 0 )
#endif
    {
      layers_SWB = 2;   move16();
    }

    pBit_SVQ = bst_buff + NBITS_MODE_R1SM_TOTLE;   

    pBit_SVQ2 = bst_buff2 + NBitsPerFrame_EL1; /*for 40 bits send in swbl2*/ 

    swbl1_encode_AVQ( (void*)w->SubEncoderSH, scoef_SWB, sFenv_SWB, 
      sFenv_SWB_unq,
      index_g, cod_Mode, pBit_SVQ, pBit_SVQ2, layers_SWB, (const Word16)scoef_SWBQ );
    softbit2hardbit (NBytesPerFrame_SWB_1, bst_buff, bpt);
    bpt += NBytesPerFrame_SWB_1;   
  }
#ifdef LAYER_STEREO
  test(); test();
  IF ((sub(w->Mode, MODE_R3sm) == 0) || (sub(w->Mode, MODE_R4ss) == 0) || (sub(w->Mode, MODE_R5ss) == 0)) {
#else
  IF (sub(w->Mode, MODE_R3sm) == 0) {
#endif
    FOR (i=0; i<shr(L_FRAME_NB, add(1, mode_enh)); i++){
      *bpt++ = bpt_enh[i];
      move16();
    }
    softbit2hardbit (NBytesPerFrame_SWB_2/2, bst_buff2+NBitsPerFrame_SWB_2/2, bpt);
    bpt += NBytesPerFrame_SWB_2/2;      
  }
#ifdef LAYER_STEREO
  IF(sub(w->Mode, MODE_R1ws) == 0)
  {
    softbit2hardbit(5, bpt_stereo_swb, bpt);
    bpt += 5;
  }   
  IF(sub(w->Mode, MODE_R2ws) == 0|| sub(w->Mode, MODE_R2ss) == 0||sub(w->Mode, MODE_R3ss) == 0||sub(w->Mode, MODE_R4ss) == 0)
  {
    softbit2hardbit(10, bpt_stereo_swb, bpt);
    bpt += 10;
  }
  IF(sub(w->Mode, MODE_R5ss) == 0)
  {
    softbit2hardbit(20, bpt_stereo_swb, bpt);
    bpt += 20;
  }
#endif
  /*****************************/
#ifdef DYN_RAM_CNT
  DYN_RAM_POP();
#endif
  /*****************************/

  return OK;
}
