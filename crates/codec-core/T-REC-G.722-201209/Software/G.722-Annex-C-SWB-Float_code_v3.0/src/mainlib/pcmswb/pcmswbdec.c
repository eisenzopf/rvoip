/* ITU G.722 3rd Edition (2012-09) */

/*--------------------------------------------------------------------------
 ITU-T G.722 Annex C (ex G.722-SWB-Float) Source Code
 Software Release 1.01 (2012-07)
 (C) 2012 France Telecom, Huawei Technologies, VoiceAge Corp., NTT.
--------------------------------------------------------------------------*/

#include "bit_op.h"
#include "errexit.h"
#include "pcmswb_common.h"
#include "softbit.h"
#include "qmfilt.h"
#include "g722.h"
#include "bwe.h"
#include "avq.h"

#define OK  0
#define NG  1

typedef struct {
  Short Mode;               /* Decoding mode */
  Short OpFs;               /* Sampling frequency */
  void* G722_SubDecoder;    /* Work space for G.722 */
  void* SubDecoderSH;       /* Work space for super-higher-band sub-decoder */
  void* SubDecoderBWE;      /* Work space for 8kbps swb extension */
  void* pQMFBuf_SWB;        /* QMF filter buffer for SWB */
  Short prev_Mode;
  Short prev2_Mode;
  Short prev_ploss_status;
  Float f_sattenu;
  Float f_sattenu1;
  Float f_sattenu3;
  Float f_prev_fenv[8];
  Short prev_bit_switch_flag, bit_switch_count;
  Short bit_switch_status;
} pcmswbDecoder_WORK;

/*----------------------------------------------------------------
Function:
PCM SWB decoder constructor
Return value:
Pointer to work space
----------------------------------------------------------------*/
void *pcmswbDecode_const(
  int mode   /* (i): Decoding mode      */
)
{
  pcmswbDecoder_WORK *w=NULL;

  /* Static memory allocation */
  w = (void *)malloc( sizeof(pcmswbDecoder_WORK) );
  if ( w == NULL )  return NULL;

  w->Mode = (Short) mode;
  w->prev_Mode = -1;
  w->prev2_Mode = -1;

  switch (w->Mode) {
  case MODE_R00wm : w->OpFs = 16000; break;
  case MODE_R0wm  : w->OpFs = 16000; break;
  case MODE_R1wm  : w->OpFs = 16000; break;
  case MODE_R1sm  : w->OpFs = 32000; break;
  case MODE_R2sm  : w->OpFs = 32000; break;
  case MODE_R3sm  : w->OpFs = 32000; break;
  default : error_exit("Decoding mode error.");
  }

  w->G722_SubDecoder = g722_decode_const();
  if (w->G722_SubDecoder == NULL)  error_exit( "G.722 decoder init error." );

  w->pQMFBuf_SWB = QMFilt_const(NTAP_QMF_SWB, fSWBQmf0, fSWBQmf1);
  if (w->pQMFBuf_SWB == NULL)  error_exit( "SWB QMF init error." );

  w->SubDecoderBWE = bwe_decode_const();
  if (w->SubDecoderBWE == NULL)  error_exit( "BWE decoder init error." );

  if( w->Mode >= MODE_R1sm )
  {
    w->SubDecoderSH = avq_decode_const();
    if (w->SubDecoderSH == NULL) error_exit( "AVQ decoder init error." );
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
int pcmswbDecode_set(
  int mode,    /* (i): Decoding mode */
  void* p_work /* (i/o): Work space  */
)
{
  pcmswbDecoder_WORK *w=(pcmswbDecoder_WORK *)p_work;

  w->Mode = mode;
  w->OpFs = 32000;

  return OK;
}

/*----------------------------------------------------------------
Function:
PCM SWB decoder destructor
Return value:
None
----------------------------------------------------------------*/
void pcmswbDecode_dest(
  void* p_work  /* (i): Work space */
)
{
  pcmswbDecoder_WORK *w=(pcmswbDecoder_WORK *)p_work;

  if (w != NULL) {
    g722_decode_dest(w->G722_SubDecoder); /* G.722       */
    QMFilt_dest(w->pQMFBuf_SWB);          /* QMF for SWB */
    bwe_decode_dest(w->SubDecoderBWE);    /* BWE for SWB */
    if (w->Mode >= MODE_R1sm)
    {
      avq_decode_dest (w->SubDecoderSH);  /* AVQ for SWB */
    }

    free( w );
  }
}

/*----------------------------------------------------------------
Function:
PCM SWB decoder reset
Return value:
OK
----------------------------------------------------------------*/
int pcmswbDecode_reset(
  void* p_work  /* (i/o): Work space */
)
{
  pcmswbDecoder_WORK *w=(pcmswbDecoder_WORK *)p_work;

  if (w != NULL) {
    QMFilt_reset(w->pQMFBuf_SWB);         /* QMF for SWB */
	bwe_decode_reset(w->SubDecoderBWE);   /* BWE for SWB */
    if (w->Mode >= MODE_R1sm)
    {
      avq_decode_reset (w->SubDecoderSH); /* AVQ for SWB */
    }

    w->f_sattenu = 0.1f;
    w->f_sattenu1 = 0.1f;
    w->f_sattenu3 = 1.0f;
    w->prev_ploss_status = 0;
    w->prev_bit_switch_flag = 0;
    w->bit_switch_count = 0;
    w->bit_switch_status = 0;
    zeroF(8, w->f_prev_fenv);
  }

  return OK;
}

void bst_frame_G722(
  unsigned char *bptframe, /*i: layered frame bitstream*/ 
  unsigned char *bptg722   /*o: g722 style bitstream, scalability by 2 samples, can be the same as the input*/
)
{
  Short decal;
  Short il;
  Short i,j;
  unsigned char *bpttmp;
  bpttmp = malloc( sizeof(Short) * L_FRAME_NB );

  zeroS(L_FRAME_NB/2, (Short *)bpttmp);

  for(j = 0; j < L_FRAME_NB; j++) {
	decal = j / 8;
	for(i = 2;  i < 8; i++) {										/*from b2 to b7*/
      il = (bptframe[decal] & 0x01) << i;							/*  bi in LSB position, shift to position i*/
	  bpttmp[j] = (unsigned char)(bpttmp[j] + il);					/*compose g722 codeword b7b6b5b4b3b2b1b0*/
	  bptframe[decal] = (unsigned char)(bptframe[decal] >> 1);		/*shift out read bit, next bit in LSB position*/
	  decal += 5;
	}
    il = (bptframe[decal] & 0x01) << 1;								/*  b1 in LSB position, shift to position 1*/
    bpttmp[j] = (unsigned char)(bpttmp[j] + il);					/*compose g722 codeword b7b6b5b4b3b2b1b0*/
    bptframe[decal] = (unsigned char)(bptframe[decal] >> 1);		/*shift out read bit, next bit in LSB position*/
    decal += 5;
    il = bptframe[decal] & 0x01;									/*  b1 in LSB position, shift to position 1*/
    bpttmp[j] = (unsigned char)(bpttmp[j] + il);					/*compose g722 codeword b7b6b5b4b3b2b1b0*/
    bptframe[decal] = (unsigned char)(bptframe[decal] >> 1);		/*shift out read bit, next bit in LSB position*/
  }
  movSS(L_FRAME_NB/2, (Short*)bpttmp, (Short*)bptg722);
  free(bpttmp);

  return;
}

/*----------------------------------------------------------------
Function:
PCM SWB decoder
Return value:
OK/NG
----------------------------------------------------------------*/
int pcmswbDecode(
  const unsigned char* bitstream,   /* (i):   Input bitstream  */
  Short*               outwave,     /* (o):   Output signal    */
  void*                p_work,      /* (i/o): Work space       */
  int                  ploss_status /* (i):   Packet-loss flag */
) 
{
  unsigned char  *bpt = (unsigned char  *)bitstream, *bptmp;
  Short i;
  Float f_SubSigSuperWideLowQMF[L_FRAME_WB];  /* 0- 8 kHz signal (low,  80 points) */
  Float f_SubSigSuperWideHighQMF[L_FRAME_WB]; /* 8-14 kHz signal (high, 80 points) */
  Float f_outwave[L_FRAME_SWB];

  pcmswbDecoder_WORK *w=(pcmswbDecoder_WORK *)p_work;

  unsigned short bst_buff[NBitsPerFrame_SWB_1];
  unsigned short bst_buff2[NBitsPerFrame_SWB_2];
  unsigned char  bst_g722[NBytesPerFrame_G722_64k*2];
  unsigned short *pBit_BWE, *pBit_SVQ, *pBit_SVQ2;
  unsigned short *pBit_wbenh;
  Short index_g, cod_Mode, T_modify_flag;
  Short bit_switch_flag;
  Short ploss_status_buff;
  Short layers_SWB;
  Float f_Tenv_SWB[SWB_TENV];
  Float f_coef_SWB[SWB_F_WIDTH];
  Float f_Fenv_SVQ[SWB_NORMAL_FENV];

  Short G722mode;
  Short nbytesPerFrame;
  Short outsig[L_FRAME_WB];
  Short mode_enh;
  Short wbenh_flag;

  /* initialize */
  layers_SWB = 0;
  bit_switch_flag = 0;
  ploss_status_buff = 0;
  wbenh_flag = 0;
  zeroS(NBitsPerFrame_SWB_1, (Short*)bst_buff);
  zeroS(NBitsPerFrame_SWB_2, (Short*)bst_buff2);
  zeroF(SWB_TENV, f_Tenv_SWB);
  zeroF(SWB_F_WIDTH, f_coef_SWB);
  zeroF(SWB_NORMAL_FENV, f_Fenv_SVQ);

  if (p_work == NULL)
  {
    return NG;
  }

  zeroF(L_FRAME_WB,f_SubSigSuperWideHighQMF);
  zeroF(L_FRAME_SWB, f_outwave);

  /* ------------------------------------------- */
  /* Super higher-band enhancement layer decoder */
  /* ------------------------------------------- */
  if (w->Mode == MODE_R1sm) { /* G.722 */
    bptmp = bpt + NBytesPerFrame_G722_56k;	
    hardbit2softbit( NBytesPerFrame_R1SM, bptmp, bst_buff );
  }
  else if (w->Mode >= MODE_R2sm)
  {
    bptmp = bpt + NBytesPerFrame_G722_64k;	
    hardbit2softbit( NBytesPerFrame_SWB_1, bptmp, bst_buff );
    if( w->Mode == MODE_R3sm ) 
    {
      bptmp += NBytesPerFrame_SWB_1 + NBytesPerFrame_SWB_2/2;	
      hardbit2softbit( NBytesPerFrame_SWB_2/2, bptmp, bst_buff2 );
    }
  }

  if (w->Mode == MODE_R00wm) {
    G722mode= 3;
    nbytesPerFrame = NBytesPerFrame_G722_48k;
  }
  else if (w->Mode == MODE_R0wm || w->Mode == MODE_R1sm) {
    G722mode= 2;
    nbytesPerFrame = NBytesPerFrame_G722_56k;
  }
  else { /* MODE_R1wm || MODE_R2sm || MODE_R3sm */
    G722mode= 1;
    nbytesPerFrame = NBytesPerFrame_G722_64k;
  }

  /* G.722 decoding */
  if (w->Mode >= MODE_R1sm)
  {
    if (bst_buff[0] == G192_BITONE && bst_buff[1] == G192_BITONE)
    {
      wbenh_flag = 0;
    }
	else
    { 
      wbenh_flag = 1;
      pBit_wbenh = &bst_buff[NBITS_MODE_R1SM_BWE];
    } 
  }

  mode_enh=3;
  if (w->Mode == MODE_R3sm)
  {
    mode_enh = G722EL1_MODE;
  }
  bptmp = bpt + (NBytesPerFrame_G722_64k+NBytesPerFrame_SWB_1); /*used only in MODE_R3sm */ 
  for (i=0 ; i<nbytesPerFrame ; i++)
  {
    bst_g722[i] = bpt[i];
  }
  for (i=nbytesPerFrame ; i<NBytesPerFrame_G722_64k ; i++)
  {
    bst_g722[i] = 0;
  }
  bst_frame_G722(bst_g722, bst_g722);

  g722_decode(G722mode, bst_g722, bptmp, mode_enh, ploss_status, outsig, w->G722_SubDecoder, &pBit_wbenh, wbenh_flag);
  bpt += nbytesPerFrame;

  /* --------------------- */
  /* Band reconstructing   */
  /* --------------------- */
  if (w->OpFs == 16000) {
    movSS(L_FRAME_WB, outsig, outwave);
  }
  else { /* w->OpFs == 32000 */
    movSF(L_FRAME_WB, outsig, f_SubSigSuperWideLowQMF);

    if (w->Mode >= MODE_R1sm || w->prev_Mode >= MODE_R1sm)
    {
      if (w->prev_Mode < 0)
      {
	    w->prev2_Mode = w->Mode;
	    w->bit_switch_count = 0;
      }
	  else if (w->Mode < MODE_R1sm  && w->prev_Mode >= MODE_R1sm)
      {
	    w->prev2_Mode = w->Mode;
	    bit_switch_flag = 1;
	    w->bit_switch_count ++;
      }
	  else if (w->Mode >= MODE_R1sm && w->prev2_Mode < MODE_R1sm)
      {
        bit_switch_flag = 2;
        w->bit_switch_count = 0;
      }
	  else
      {
	    w->bit_switch_count = 0;
      }

      /* BWE decoding from SWBL0 */
      pBit_BWE = bst_buff;
	  cod_Mode = NORMAL;

      if (w->Mode >= MODE_R1sm)
      {
	    unsigned short *pBit = pBit_BWE;
	    cod_Mode = GetBit(&pBit, 2);
      }
	  ploss_status_buff = ploss_status;
      if( w->prev_ploss_status == 1 && cod_Mode <= 1 && w->Mode != MODE_R1sm )
      {
	    ploss_status = 1;
      }

      T_modify_flag = bwe_dec_freqcoef(&pBit_BWE, f_SubSigSuperWideLowQMF, w->SubDecoderBWE,
        &cod_Mode, f_Tenv_SWB, f_coef_SWB, &index_g, f_Fenv_SVQ,
        ploss_status, bit_switch_flag, w->prev_bit_switch_flag);

      /* AVQ decoding from SWBL1&2 */
      if (w->Mode >= MODE_R2sm)
      {
	    if (ploss_status == 0 && w->bit_switch_status == 0)
        {
		  layers_SWB = 1;
		  if (w->Mode == MODE_R3sm)
		  {
		    layers_SWB = 2;
		  }
		  pBit_SVQ = bst_buff + NBITS_MODE_R1SM_TOTLE;
		  pBit_SVQ2 = bst_buff2;

		  swbl1_decode_AVQ((void*)w->SubDecoderSH, pBit_SVQ, pBit_SVQ2, (const Float*)f_Fenv_SVQ,
            f_coef_SWB, index_g, cod_Mode, layers_SWB);
		  w->prev_ploss_status = 0;
	    }
	    else
	    {
		  bwe_avq_buf_reset(w->SubDecoderSH);
		  w->prev_ploss_status = 1;
		  if (ploss_status_buff == 0 )
		  {
		    AVQ_state_dec *wtmp = w->SubDecoderSH;
		    w->prev_ploss_status = 0;
		    wtmp->pre_cod_Mode = cod_Mode;
		  }
        }
      }

      if (bit_switch_flag == 1)
      {
	    for (i=0 ; i<60 ; i++)
        {
          f_coef_SWB[i] = f_coef_SWB[i] * w->f_sattenu3;
	    }
	    if (w->bit_switch_count > 200)
        {
          w->f_sattenu3 -= 0.01f;
	    }
	    w->f_sattenu3 = f_max(w->f_sattenu3, 0.0f);
	  }
      else
      {
	    w->f_sattenu3 = 1.0f;
      }

      if (bit_switch_flag == 2)
      {
	    for (i=0 ; i<60 ; i++)
        {
		  f_coef_SWB[i] = f_coef_SWB[i] * w->f_sattenu;
	    }
	    w->f_sattenu += 0.02f;
	    w->prev2_Mode = MODE_R0wm;
        if (w->f_sattenu > 1.0f)
	    {
		  w->f_sattenu = 0.1f;
		  bit_switch_flag = 0;
		  w->prev2_Mode = w->Mode;
	    }
      }

      if (bit_switch_flag == 0) 
      {
	    if (w->prev_bit_switch_flag == 1)
	    {
          w->f_sattenu1 = 0.1f;
	    }
      }
	  else
      {
	    if (w->f_sattenu1 < 1.0f)
	    {
		  for (i=0 ; i<60 ; i++)
          {
		    f_coef_SWB[i] = f_coef_SWB[i] * w->f_sattenu1;
		  }
		  w->f_sattenu1 += 0.02f;
	    }
      }

      /* BWE-based post-processing */
	  bwe_dec_timepos( cod_Mode, f_Tenv_SWB, f_coef_SWB, f_SubSigSuperWideHighQMF,
        w->SubDecoderBWE, ploss_status, T_modify_flag );
    }
    else
    {
	  bwe_dec_update( 
	    f_SubSigSuperWideLowQMF,    /* (i): Input lower-band WB signal */
	    w->SubDecoderBWE            /* (i/o): Pointer to work space        */
	  );
    }
    QMFilt_syn(L_FRAME_WB, f_SubSigSuperWideLowQMF, f_SubSigSuperWideHighQMF, f_outwave, w->pQMFBuf_SWB );

    movFS(L_FRAME_SWB, f_outwave, outwave);

    w->bit_switch_status = 0;
    if (w->Mode < MODE_R1sm)
    {
	  w->bit_switch_status = 1;
    }

    if( bit_switch_flag == 1 )
    {
	  w->Mode = MODE_R1sm;
    }

    w->prev_Mode = w->Mode;
    w->prev_bit_switch_flag = bit_switch_flag;

    movF(8, f_Fenv_SVQ, w->f_prev_fenv);
  }

  return OK;
}
