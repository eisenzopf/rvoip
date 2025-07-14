/* ITU G.722 3rd Edition (2012-09) */

/*--------------------------------------------------------------------------
 ITU-T G.722 Annex C (ex G.722-SWB-Float) Source Code
 Software Release 1.01 (2012-07)
 (C) 2012 France Telecom, Huawei Technologies, VoiceAge Corp., NTT.
--------------------------------------------------------------------------*/

#include "errexit.h"
#include "pcmswb_common.h"
#include "softbit.h"
#include "prehpf.h"
#include "qmfilt.h"
#include "g722.h"
#include "bwe.h"
#include "avq.h"

#define OK  0
#define NG  1

/* High-pass filter cutoff definition */
#define FILT_NO_8KHZ_INPUT   5
#define FILT_NO_16KHZ_INPUT  6
#define FILT_NO_32KHZ_INPUT  7

typedef struct {
  Short Mode;               /* Encoding mode */
  Short OpFs;               /* Sampling frequency */
  Float f_DCBuf[QMF_DELAY_WB];
  void* pHpassFiltBuf;      /* High-pass filter buffer */
  void* G722_SubEncoder;    /* Work space for G.722 */
  void* SubEncoderSH;       /* Work space for super-higher-band sub-encoder */
  void* SubEncoderBWE;      /* Work space for 8kbps swb extension to G.722 */
  void* pQmfBuf_SWB;        /* QMF buffer for SWB input */
} pcmswbEncoder_WORK;

/*----------------------------------------------------------------
Function:
PCM SWB encoder constructor
Return value:
Pointer to work space
----------------------------------------------------------------*/
void *pcmswbEncode_const(
  unsigned short sampf, /* (i): Input sampling rate (Hz) */
  int mode              /* (i): Encoding mode            */
)
{
  pcmswbEncoder_WORK *w=NULL;

  /* Static memory allocation */
  w = (void *)malloc(sizeof(pcmswbEncoder_WORK));
  if (w == NULL)  return NULL;

  w->Mode = mode;
  w->OpFs = 32000; /* Input sampling rate is 32kHz in default */
  if (sampf == 16000)
  {
    w->OpFs = 16000; /* Input sampling rate is 16 kHz */
  }

  zeroF(QMF_DELAY_WB, w->f_DCBuf);
  if (w->Mode < 0 || w->Mode> 5) {
    error_exit( "Encoding mode error." );
  }

  w->pHpassFiltBuf = highpass_1tap_iir_const();
  if (w->pHpassFiltBuf == NULL)  error_exit( "HPF init error." );

  w->pQmfBuf_SWB = QMFilt_const(NTAP_QMF_SWB, fSWBQmf0, fSWBQmf1);
  if (w->pQmfBuf_SWB == NULL)  error_exit( "SWB QMF init error." );

  w->G722_SubEncoder = fl_g722_encode_const();
  if (w->G722_SubEncoder == NULL)  error_exit( "G.722 encoder init error." );

  w->SubEncoderBWE = bwe_encode_const();
  if (w->SubEncoderBWE == NULL)   error_exit( "BWE encoder init error." );

  if(w->Mode == MODE_R1sm  || w->Mode == MODE_R2sm || w->Mode == MODE_R3sm)
  {
    w->SubEncoderSH = avq_encode_const();
    if (w->SubEncoderSH == NULL) error_exit( "AVQ encoder init error." );
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
    highpass_1tap_iir_dest( w->pHpassFiltBuf ); /* HPF         */
    QMFilt_dest( w->pQmfBuf_SWB );              /* QMF for SWB */
	fl_g722_encode_dest(w->G722_SubEncoder);    /* G.722       */
    bwe_encode_dest( w->SubEncoderBWE );        /* BWE for SWB */
    if( w->Mode == MODE_R1sm  || w->Mode == MODE_R2sm || w->Mode == MODE_R3sm )
    {
      avq_encode_dest (w->SubEncoderSH);        /* AVQ for SWB */
    }

    free( w );
  }
}

/*----------------------------------------------------------------
Function:
PCM SWB encoder reset
Return value:
OK
----------------------------------------------------------------*/
int  pcmswbEncode_reset(
                           void*  p_work   /* (i/o): Work space */
                           )
{
  pcmswbEncoder_WORK *w=(pcmswbEncoder_WORK *)p_work;

  if( w != NULL )
  {
    highpass_1tap_iir_reset(w->pHpassFiltBuf); /* HPF         */
    QMFilt_reset( w->pQmfBuf_SWB );            /* QMF for SWB */
	fl_g722_encode_reset(w->G722_SubEncoder);  /* G.722       */
	bwe_encode_reset( w->SubEncoderBWE );      /* BWE for SWB */
    if( w->Mode == MODE_R1sm  || w->Mode == MODE_R2sm || w->Mode == MODE_R3sm)
    {
      avq_encode_reset (w->SubEncoderSH);      /* AVQ for SWB */
    }
  }

  return OK;
}

void bst_G722_frame(
  unsigned char *bptg722, /*i: g722 style bitstream, scalability by 2 samples       */
  unsigned char *bptframe /*o: layered frame bitstream, can be the same as the input*/
)
{
  /*  write [ b2*n, b3*n, b4*n, b5*n, b6*n, b7*n, b1*n, b0*n]  to enable truncation of G.722 g192 frames */
  Short il;
  Short decal;
  Short j, i;
  unsigned char *bpttmp;
  bpttmp = malloc(sizeof(Short) * L_FRAME_NB);

  zeroS(L_FRAME_NB/2 , (Short*)bpttmp);

  for (j=0 ; j<L_FRAME_NB ; j++)
  {
    decal = j / 8; /* shr(j<3) */

    for (i=2 ; i<8 ; i++)
    { /*from b2 to b7*/
      il = (bptg722[j] & (0x01 << i) ) << (7-i);
      bpttmp[decal] = (unsigned char)( il + (bpttmp[decal] >> 1));
      decal += 5;
    }
	il = ( (bptg722[j] & 0x02) << 6 );
	bpttmp[decal] = (unsigned char)( il + (bpttmp[decal] >>1));
	decal +=  5;
	il = ( (bptg722[j] & 0x01) << 7 );  /* i = 0 : b0,  left aligned*/
    bpttmp[decal] = (unsigned char)(il + (bpttmp[decal] >> 1)); 
  }

  movSS(L_FRAME_NB/2, (Short *)bpttmp, (Short *)bptframe);
 
  free(bpttmp);

  return;
}

/*----------------------------------------------------------------
Function:
PCM SWB encoder
Return value:
OK/NG
----------------------------------------------------------------*/
int pcmswbEncode(
  const Short*   inwave,
  unsigned char* bitstream,
  void*          p_work
) 
{
  unsigned char *bpt = bitstream;
  Short i;
  Float f_SubSigSuperWideLow[L_FRAME_WB];  /* 0- 8 kHz signal (80 points) */
  Float f_SubSigSuperWideHigh[L_FRAME_WB]; /* 8-14 kHz signal (80 points) */
  Float f_SubSigSuperWideHigh_temp[QMF_DELAY_WB];
  Float f_SigInQMF[L_FRAME_SWB];

  pcmswbEncoder_WORK *w=(pcmswbEncoder_WORK *)p_work;
  BWE_state_enc *enc_st = (BWE_state_enc *)w->SubEncoderBWE;

  unsigned short bst_buff[NBitsPerFrame_SWB_1];
  unsigned short bst_buff2[NBitsPerFrame_SWB_2];
  unsigned short *pBit_BWE, *pBit_SVQ, *pBit_SVQ2;
  unsigned short *pBit_wbenh;
  Short transi;
  Short index_g, cod_Mode, T_modify_flag = 0;
  Short layers_SWB; 
  Float f_Fenv_SWB_unq[SWB_NORMAL_FENV];
  Float f_tEnv[SWB_TENV];
  Float f_coef_SWB[SWB_F_WIDTH];
  Float f_Fenv_SWB[SWB_NORMAL_FENV];

  unsigned char bpt_enh[L_FRAME_WB/8]; /* 2 bits/sample at 4 kHz */  
  Short G722mode, localmode;
  Short nbytesPerFrame;
  Short insig[L_FRAME_WB];
  Short mode_enh=3;
  Short wbenh_flag;
  Short nbbytes_g722[4] = {-1, NBytesPerFrame_G722_64k, NBytesPerFrame_G722_56k, NBytesPerFrame_G722_48k};

  /* initialize */
  zeroS(NBitsPerFrame_SWB_1, (Short*)bst_buff);
  zeroS(NBitsPerFrame_SWB_2, (Short*)bst_buff2);
  zeroF(SWB_NORMAL_FENV, f_Fenv_SWB_unq);
  zeroF(SWB_TENV, f_tEnv);
  zeroF(SWB_F_WIDTH, f_coef_SWB);
  zeroF(SWB_NORMAL_FENV, f_Fenv_SWB);

  if (p_work == NULL)
  {
    return NG;
  }

  /* ------------------------------- */
  /* Pre-processing & band splitting */
  /* ------------------------------- */
  if( w->OpFs == 16000 ){ /* Wideband input */
    /* High-pass filtering */
    highpass_1tap_iir(FILT_NO_16KHZ_INPUT, L_FRAME_WB, (Short *)inwave, f_SigInQMF, w->pHpassFiltBuf);
  }
  else{ /* w->OpFs == 32000 */  /* Super wideband input */
    /* High-pass filtering */
    highpass_1tap_iir(FILT_NO_32KHZ_INPUT, L_FRAME_SWB, (Short *)inwave, f_SigInQMF, w->pHpassFiltBuf);

    /* Band splitting with QMF for SWB */
    QMFilt_ana(L_FRAME_SWB, f_SigInQMF, f_SubSigSuperWideLow, f_SubSigSuperWideHigh, w->pQmfBuf_SWB);
  }

  movF(QMF_DELAY_WB, &f_SubSigSuperWideHigh[L_FRAME_WB-QMF_DELAY_WB], f_SubSigSuperWideHigh_temp);
  movF_bwd(L_FRAME_WB-QMF_DELAY_WB, f_SubSigSuperWideHigh+L_FRAME_WB-1-QMF_DELAY_WB, f_SubSigSuperWideHigh+L_FRAME_WB-1);
  movF(QMF_DELAY_WB, w->f_DCBuf, f_SubSigSuperWideHigh);
  movF(QMF_DELAY_WB, f_SubSigSuperWideHigh_temp, w->f_DCBuf);

  /* ------------------------------------------------ */
  /* G.722 encoder including enhancement layer coding */
  /* ------------------------------------------------ */
  if (w->Mode == MODE_R00wm) {
  	G722mode = 3;
	nbytesPerFrame = NBytesPerFrame_G722_48k;
  }
  else if (w->Mode == MODE_R0wm || w->Mode == MODE_R1sm) {
	G722mode = 2;
    nbytesPerFrame = NBytesPerFrame_G722_56k;
  }
  else { /* MODE_R1wm || MODE_R2sm || MODE_R3sm */
  	G722mode = 1;
	nbytesPerFrame = NBytesPerFrame_G722_64k;
  }

  if (w->OpFs == 16000) {
    movFS(L_FRAME_WB, f_SigInQMF, insig);
  }
  else { /* w->OpFs == 32000 */
    movFS(L_FRAME_WB, f_SubSigSuperWideLow, insig);
  }

  localmode = G722mode; 
  if (G722mode == 1)
  {
    localmode = 2;
  }
  wbenh_flag = 0;

  if (w->Mode == MODE_R1sm || w->Mode == MODE_R2sm || w->Mode == MODE_R3sm)
  {
    T_modify_flag = Icalc_tEnv(f_SubSigSuperWideHigh, f_tEnv, &transi, enc_st->preMode, (void*)enc_st);
    if( transi != 1 && enc_st->preMode != TRANSIENT )
    {
      wbenh_flag = 1;
      pBit_wbenh = &bst_buff[NBITS_MODE_R1SM_BWE];
    }
  }

  if (w->Mode == MODE_R3sm)
  {
    mode_enh=G722EL1_MODE;
  }
  g722_encode(G722mode, localmode, insig, bpt, bpt_enh, mode_enh, w->G722_SubEncoder, wbenh_flag, &pBit_wbenh);

  bst_G722_frame(bpt, bpt);
  bpt += nbbytes_g722[G722mode];

  /* ------------------------------------------- */
  /* Super-higher-band enhancement layer encoder */
  /* ------------------------------------------- */
  if (w->Mode == MODE_R1sm)
  {
    /* BWE encoding for SWBL0 */
    pBit_BWE = bst_buff;
    bwe_enc( f_SubSigSuperWideHigh, &pBit_BWE, w->SubEncoderBWE, f_tEnv, transi,
      &cod_Mode, f_Fenv_SWB, f_coef_SWB, &index_g, T_modify_flag, f_Fenv_SWB_unq );

    softbit2hardbit (NBytesPerFrame_R1SM, bst_buff, bpt);
    bpt += NBytesPerFrame_SWB_0;
  }
  else if (w->Mode == MODE_R2sm || w->Mode == MODE_R3sm)
  {
    /* BWE encoding for SWBL0 */
    pBit_BWE = bst_buff;
    bwe_enc(f_SubSigSuperWideHigh, &pBit_BWE, w->SubEncoderBWE, f_tEnv, transi,
      &cod_Mode, f_Fenv_SWB, f_coef_SWB, &index_g, T_modify_flag, f_Fenv_SWB_unq);

    /* AVQ encoding for SWBL1&2 */
    layers_SWB = 1;
    if( w->Mode == MODE_R3sm )
    {
      layers_SWB = 2;
    }

    pBit_SVQ = bst_buff + NBITS_MODE_R1SM_TOTLE;
    pBit_SVQ2 = bst_buff2 + NBitsPerFrame_EL1;

	swbl1_encode_AVQ( (void*)w->SubEncoderSH, f_coef_SWB, f_Fenv_SWB, f_Fenv_SWB_unq,
      index_g, cod_Mode, pBit_SVQ, pBit_SVQ2, layers_SWB );
    softbit2hardbit (NBytesPerFrame_SWB_1, bst_buff, bpt);
    bpt += NBytesPerFrame_SWB_1;
  }

  if (w->Mode == MODE_R3sm)
  {
    for (i=0; i<(L_FRAME_NB / Pow(2.0f,(Float)(1+mode_enh))); i++)
    {
      *bpt++ = bpt_enh[i];
    }
    softbit2hardbit (NBytesPerFrame_SWB_2/2, bst_buff2+NBitsPerFrame_SWB_2/2, bpt);
    bpt += NBytesPerFrame_SWB_2/2;		
  }

  return OK;
}
