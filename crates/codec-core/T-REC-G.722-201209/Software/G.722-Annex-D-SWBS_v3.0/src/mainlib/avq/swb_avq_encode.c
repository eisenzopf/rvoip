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

#include "bit_op.h"
#include "bwe.h"
#include "avq.h"
#include "math_op.h"
#include "rom.h"

/*****************************/
#ifdef DYN_RAM_CNT
#include "dyn_ram_cnt.h"
#endif
/*****************************/

/*------------------------------------------------------------------------*
* Prototypes
*------------------------------------------------------------------------*/      
Word32  Sum_vect_E8(const Word16 *vec);   /* DEFINED IN fec_low_band.c */

static void getIndexBitstream( Word16 nbBit, Word16 val, Word16 *nbBitCum, Word32 *index);
static void compute_sratio (Word16 *sEnv_BWE, Word16 *sratio, Word16 sYfb_Q);
static Word16 compute_ksm(Word16 *Yb, Word16 *sy_s_abs, Word16 *sord_b, 
                          AVQ_state_enc *w_AVQ_state_enc); 
static void compute_mdct_err (Word16 *sy_s_abs, Word16 *smdct_err, 
                              Word16 *scoef_SWB, Word16 *senv_BWE_err, Word16 *senv_BWE);
static Word16 detectPbZeroBand_flg0(const Word16 *sykr, const Word16 *sratio_fEnv, 
                                    const Word16 *ord_bands, Word16 *bandZero, Word16 nbBand0, Word16 cnt_detzer);
static Word16 detectPbZeroBand_flg1(Word16 nbBandZero, Word16 *bandZero, 
                                    Word16 unbits_L1, Word16 *sratio, Word16 *nbBits);
static void encoder_SWBL1L2_AVQ(const Word16 *smdct_coef, UWord16 **pBst_L1,  UWord16 **pBst_L2, const Word16 layers, 
                                Word16 *avqType, Word16 *scoef_SWB_AVQ,  Word16 *unbits_L1, Word16 *unbits_L2);
static Word16 ggain_adj(Word16  *bandL1, Word16 nbBandL1, Word16 *sx, Word16 *sqx, Word16 index_g_5bit, Word16 *sGopt_Q);
static Word16 cod_emb_fgain (Word16 index_g_5bit, Word16 *sGopt);
static void encoder_coef_SWB_AVQ_adj(const Word16 coef_SWB[], Word16 bandL[], Word16 nbBand, Word16 coef_SWB_AVQ[], 
                                     Word32 *indexL_, Word16 *nbBitsL_, Word16 nbBitsTot);
static Word16 minDiff0Array16( Word16 n, Word16 x, Word16 *y, Word32 *Lmin_dist);
static Word16 compute_errGradNormL1(Word16 *x, Word16 *xq, Word16 *sgrad, Word16 max_idx);
static void bandNormalize_Order( const Word16 *sykr, Word16 *smdct_coef, const Word16 *senv_BWE, const Word16 *ord_bands);
static void globalGainAdj (Word16 *avqType, Word16 *scoef_SWB_AVQ, Word16 *scoef_SWB_AVQ_abs, Word16 *senv_BWE);
static Word16 invEnv_BWE(Word16 sEnv, Word16 expx, Word16 *exp_num);
static Word16 Compute_Corr(const Word16 vec_base[], const Word16 vec_fill[]);
static Word16 getParamFillBand(Word16 *svec_base, Word16 *vec_fill, Word16 expx, Word16 *ind_corr_max );
static void getBaseSpectrum_flg1(Word16 *avqType, Word16 *senv_BWE, Word16 *sEnv_BWE, Word16 *svec_base, Word16 *scoef_SWB_AVQ_abs, 
                                 Word16 *scoef_SWB_AVQ, Word16 *scoef_SWB, Word16 scoef_SWBQ);
static void getVecToFill_flg1( Word16 senv_BWE, Word16 *scoef_SWB, Word16 *vecToFill);
static Word16 fillZeroBands_flg1(Word16 *avqType, Word16 *iZero, Word16 *scoef_SWB, Word16 *scoef_SWB_AVQ_abs, 
                                 Word16 *senv_BWE, Word16 *svec_base);
static Word16 fillZeroBands_flg0(Word16 Qval, Word16 *avqType, Word16 *smdct_coef_nq, Word16 *smdct_coef_avq, Word16 *svec_base);
static Word16 getSign(Word16 *ptr0, Word16 *ptr1, Word16 nbBit);
static void getSignInfo( Word16 *avqType, const Word16 *scoef_SWB, Word16 *smdct_coef, Word32 *indexL1, Word16 *nbBitsL1, 
                        Word16 *nbBitsRestL1, Word32 *indexL2, Word16 *nbBitsL2, Word16 *nbBitsRestL2, Word16 nbBitsRest);

/* Constructor for AVQ encoder */
void* avq_encode_const (void)
{
  AVQ_state_enc *enc_st = NULL;

  enc_st = (AVQ_state_enc *) malloc (sizeof(AVQ_state_enc));
  if (enc_st == NULL) return NULL;

#ifdef DYN_RAM_CNT
#ifdef MEM_STT
  {
    UWord32 ssize = 0;
    ssize += (UWord32) (5*SIZE_Word16);
    DYN_RAM_PUSH(ssize, "dummy");
  }
#endif
#endif

  avq_encode_reset ((void *)enc_st);

  return (void *) enc_st;
}

void avq_encode_dest (void *work)
{
  AVQ_state_enc *enc_st = (AVQ_state_enc *)work;

  if (enc_st != NULL)
  {
    free (enc_st);
  }

#ifdef DYN_RAM_CNT
#ifdef MEM_STT
  DYN_RAM_POP();
#endif
#endif
}

Word16 avq_encode_reset (void *work)
{
  AVQ_state_enc *enc_st = (AVQ_state_enc *) work;

#ifdef DYN_RAM_CNT
  DYN_RAM_PUSH((UWord32) SIZE_Ptr, "dummy");
#endif

  if (enc_st != NULL)
  {
    /* initialize each member */
    zero16(sizeof(AVQ_state_enc)/2, (Word16 *)work);
  }

#ifdef DYN_RAM_CNT
  DYN_RAM_POP();
#endif

  return ENCODER_OK;
}

static void encoder_SWBL1L2_AVQ( 
                                const Word16 *smdct_coef,    /* i: MDCT coefficients to encode     */
                                UWord16 **pBst_L1,           /* i: pointer to L1 bitstream buffer        */
                                UWord16 **pBst_L2,           /* i: pointer to L2 bitstream buffer*/
                                const Word16 layers,         /* i: number of swb layers encoded  */
                                Word16 avqType[],            /* o: Output vector signalising zero bands */
                                Word16 *scoef_SWB_AVQ,       /* o: locally decoded MDCT coefs. */
                                Word16 *unbits_L1,
                                Word16 *unbits_L2            /* i: Q12 */
                                )
{
  Word16 ib, i;
  Word16 smdct_coef_norm_L1[(WIDTH_BAND+1)*N_SV_L1], smdct_coef_norm_L2[(WIDTH_BAND+1)*N_SV_L2];
  Word16 smdct_coef_L2[WIDTH_BAND*N_SV_L2];
  Word16 *bandL1, *bandL2, *bandZero, bandTmp[N_SV];
  Word16 nbBandZero;
  Word16 *ptr0, *ptr1;
  Word16 *bandLx;

  /*****************************/
#ifdef DYN_RAM_CNT
  DYN_RAM_PUSH((UWord32) ((3 + (N_SV_L1 + N_SV_L2) * (WIDTH_BAND + 1) + WIDTH_BAND * N_SV_L2 + N_SV) * SIZE_Word16 + 6 * SIZE_Ptr), "dummy");
#endif
  /*****************************/  

  bandL1 = avqType+3;
  bandL2 = bandL1 + N_SV_L1;
  bandZero = bandL2 + N_SV_L2+2;
  /* SWBL1 AVQ encoder */
  AVQ_Cod( (Word16 *)smdct_coef, smdct_coef_norm_L1, N_BITS_AVQ_L1, N_SV_L1 );
  *unbits_L1 = AVQ_Encmux_Bstr( smdct_coef_norm_L1, pBst_L1, N_BITS_AVQ_L1, N_SV_L1 );

  /* get bands coded with L1 and zero bands and local decoding of SWBL1 */
  FOR(i=0; i<N_SV; i++)
  {
    bandTmp[i] = i; move16();
  }
  bandLx = bandL2;
  if (sub(layers, 1) == 0)
  {
    bandLx = bandZero;
  }
  avqType[0] = getBandLx_decodAVQ(smdct_coef_norm_L1, scoef_SWB_AVQ, bandTmp, N_SV_L1, bandL1, bandLx); 
  move16();

  nbBandZero = sub(N_SV_L1, avqType[0]);
  IF(sub(layers, 2) == 0)
  {
    /* form bands to be coded with L2 */
    mov16(sub(N_SV_L2,nbBandZero), &bandTmp[N_SV_L1], &bandL2[nbBandZero]);
    ptr1= smdct_coef_L2;
    FOR(ib= 0; ib<N_SV_L2; ib++)
    {
      ptr0 = (Word16*)smdct_coef + shl(bandL2[ib],3);

      mov16_8(ptr0, ptr1);

      ptr1 += WIDTH_BAND;
    }
    /* SWBL2 AVQ encoder */
    AVQ_Cod( (Word16 *)smdct_coef_L2, smdct_coef_norm_L2, N_BITS_AVQ_L2, N_SV_L2 );
    *unbits_L2 = AVQ_Encmux_Bstr(smdct_coef_norm_L2, pBst_L2, N_BITS_AVQ_L2, N_SV_L2);
    /* get bands coded with L2 and zero bands and local decoding of SWBL2 */
    avqType[1]= getBandLx_decodAVQ(smdct_coef_norm_L2, scoef_SWB_AVQ, bandL2, N_SV_L2, bandL2, bandZero);
    move16();
    mov16(sub(N_SV-N_SV_L2,avqType[0]), &bandTmp[add(N_SV-N_SV_L1-1,avqType[0])], &bandZero[sub(N_SV_L2,avqType[1])]);
    nbBandZero = sub(N_SV, add(avqType[0], avqType[1]));
  }
  ELSE {
    nbBandZero = sub(N_SV, avqType[0]);
    mov16(N_SV-N_SV_L1, &bandTmp[N_SV_L1], &bandZero[sub(N_SV_L1,avqType[0])]);
    *unbits_L2 = 0;
  }
  FOR(ib=0; ib<nbBandZero; ib++) 
  {
    ptr1 = scoef_SWB_AVQ + shl(bandZero[ib],3);

    zero16_8(ptr1);
  }
  avqType[2] = nbBandZero; move16();
  /*****************************/
#ifdef DYN_RAM_CNT
  DYN_RAM_POP();
#endif
  /*****************************/ 
  return;
}

static Word16 getParamFillBand (Word16 *svec_base, Word16 *vec_fill, Word16 expx, 
                                Word16 *Gain16)
{
  Word16 ind_corr_max ; 
  /*****************************/
#ifdef DYN_RAM_CNT
  DYN_RAM_PUSH((UWord32)  (1 * SIZE_Word16 + 0 * SIZE_Word32), "dummy");
#endif
  /*****************************/  
  ind_corr_max  = Compute_Corr( svec_base, vec_fill);
  /* reconstruct the zero subband */
  IF( sub(ind_corr_max,CORR_RANGE_L1) < 0)
  {
      *Gain16 = computeGainFillBand(&svec_base[ind_corr_max], expx); move16();
  }
  
#ifdef DYN_RAM_CNT
  DYN_RAM_POP();
#endif
  return (ind_corr_max);
}

static Word16 Compute_Corr(
                           const Word16 vec_base[],
                           const Word16 vec_fill[]
                           )
{
  Word16 i, ind_max;
  Word32 corr, corr_max;
  const Word16 *ptr;
  /*****************************/
#ifdef DYN_RAM_CNT
  DYN_RAM_PUSH((UWord32) (2 * SIZE_Word16 + 2 * SIZE_Word32+ SIZE_Ptr), "dummy");
#endif
  /*****************************/  
  /* compute correlations */
  ptr = vec_base;
  corr_max = 0;                  move32();
  ind_max = CORR_RANGE_L1;                   move16();
  FOR( i=0; i<CORR_RANGE_L1; i++ )
  {
    corr = L_mac_Array8(0, (Word16*)ptr, (Word16*)vec_fill);

    ptr++;
    if( L_sub(corr, corr_max) > 0)
    {
      ind_max = i;           move16();
    }
    corr_max  = L_max(corr_max , corr);
  }
  /*****************************/
#ifdef DYN_RAM_CNT
  DYN_RAM_POP();
#endif
  /*****************************/ 

  return( ind_max );
}

static void compute_sratio (Word16 *sEnv_BWE, Word16 *sratio, Word16 sYfb_Q)
{
  Word16 i, norm_ratio, stmp, tmp;
#ifdef DYN_RAM_CNT
  DYN_RAM_PUSH((UWord32) (4 * SIZE_Word16), "dummy");
#endif

  FOR (i=0; i<N_SV; i++)
  {
    if (sratio[i] == 0){ sratio[i] = 1; move16();}
    norm_ratio = norm_s (sratio[i]); 
    stmp = shl (sratio[i], norm_ratio); /* Q(12+norm) */
    stmp = div_s (16384, stmp); /* Q(17-norm):15+14-(12+norm) */
    norm_ratio = add (sub (sYfb_Q, norm_ratio), 2); ;
    tmp = sub(mult_r(sEnv_BWE[i], stmp),shl (1, norm_ratio)); /* need to be checked */ /* Q(norm[i]):sYfb_Q-norm+2=sYfb_Q+17-norm+1-16 */
    sratio[i] = shr (tmp, sub(norm_ratio, 12)); move16 ();
  }
#ifdef DYN_RAM_CNT
  DYN_RAM_POP();
#endif
  return;
}

/* calculate ksm and absolute value of mdct coefficients */
static Word16 compute_ksm(Word16 *Yb, Word16 *sy_s_abs, Word16 *sord_b, 
                          AVQ_state_enc *w_AVQ_state_enc) 
{
  Word16 k, i, j, flg_bit;
  Word16 *ptr;
  Word32 L_sksm;
#ifdef DYN_RAM_CNT
  DYN_RAM_PUSH((UWord32) (4 * SIZE_Word16 + SIZE_Word32 + SIZE_Ptr), "dummy");
#endif

  /* calculate abs. value of Yb */
  abs_array(Yb, sy_s_abs, SWB_F_WIDTH);
  k = 0; move16();
  ptr = sy_s_abs;
  FOR (i=0; i<N_SV; i++)
  {    
    IF ( sub(sord_b[i], TH_ORD_B) < 0 )
    { 
      FOR (j=0; j<WIDTH_BAND; j++)
      {
        if ( sub(ptr[j], 2048) < 0 ) /* Q(12) */
        {
          k = add(k, 512); /* Q(9) */
        }
      }
    }
    ptr += WIDTH_BAND;
  }
  flg_bit = 0; move16();
  /* ksm *= (1.f-SMOOTH_K);         */
  j = mult(w_AVQ_state_enc->sksm, 22938);  /* Q(9): Q(9) + Q(15) + 1 - 16 */
  /* ksm += (SMOOTH_K * (float) k); */
  L_sksm = L_shl(j, 16); /* Q(25): Q(9) + 16 */
  w_AVQ_state_enc->sksm = mac_r(L_sksm, k, 9830);/* Q(9): Q(9) + Q(15) + 1 - 16 */
  /* select encoding mode */
  /* if ( mnl == LOW_LEVEL_NUM_MIN ) */

  IF (sub(w_AVQ_state_enc->sksm, 7680) <= 0)
  {
    /* flag  = 1 */
    flg_bit = add(flg_bit, 1);
  }
  ELSE
  {
    if ( sub(w_AVQ_state_enc->sksm, 10240)< 0)
    {
      /* flag  = 1 */
      flg_bit = w_AVQ_state_enc->smnl; move16();
    }
  }
  w_AVQ_state_enc->smnl = flg_bit ;           move16();
#ifdef DYN_RAM_CNT
  DYN_RAM_POP();
#endif
  return (flg_bit);
}
static void compute_mdct_err (Word16 *sy_s_abs, Word16 *smdct_err, Word16 *scoef_SWB,
                              Word16 *senv_BWE_err, Word16 *senv_BWE)
{
  Word16 *p_sy, *p_se, *ptr0;
  Word16 i, j, stmp, tmp;
#ifdef DYN_RAM_CNT
  DYN_RAM_PUSH((UWord32) (4 * SIZE_Word16 + 3 * SIZE_Ptr), "dummy");
#endif

  p_sy = sy_s_abs;
  p_se = smdct_err;
  ptr0 = scoef_SWB;
  FOR (i=0; i<N_SV; i++)
  {
    stmp = mult(16384, senv_BWE[i]);
    FOR (j=0; j<WIDTH_BAND; j++)
    {
      /* calculate MDCT error */
      tmp = sub( *p_sy++, stmp); 
      /* delete negative component */
      tmp = s_max (0, tmp);
      if (*ptr0 < 0)
      {
        tmp = negate(tmp);
      }
      *p_se++ = tmp; move16();
      ptr0++;
    }
    /* calculate Fenv_BWE_err */
    /* ---------------------- */
    senv_BWE_err[i] = mult(19661, senv_BWE[i]); /* Q(12): Q(15) + Q(12) + 1 - 16 */ move16();
  }
#ifdef DYN_RAM_CNT
  DYN_RAM_POP();
#endif
  return;
}

static Word16 detectPbZeroBand_flg1(Word16 nbBandZero, Word16 *bandZero, 
                                    Word16 unbits_L1, Word16 *sratio, Word16 *nbBits)
{
  Word16 detprob_flg,  smax_ratio;

  Word16 ib, nb;
#ifdef DYN_RAM_CNT
  DYN_RAM_PUSH((UWord32) (4 * SIZE_Word16), "dummy");
#endif

  /* determine on how many bits detprob_flg can be written; nb_bits = 0, 1, or 2 */
  nb = getNbBitdetprob_flg (unbits_L1);
  detprob_flg = 0;          move16();
  IF(nb >0) 
  {
    smax_ratio = sratio[bandZero[0]];      move16();
    FOR( ib=1; ib<nbBandZero; ib++ )
    {
      smax_ratio = s_max(sratio[bandZero[ib]], smax_ratio);
    }

    IF( sub(smax_ratio, 8192) > 0 )
    {
      detprob_flg = add(1, detprob_flg);        
      IF( sub(smax_ratio, 16384) > 0 )
      {
        detprob_flg = add(1, detprob_flg);        
        if( sub(smax_ratio, 32767) >= 0 ){
          detprob_flg = add(1, detprob_flg);        
        }
      }
    }
    if( sub(nb,1) == 0) 
    {
      detprob_flg = s_min(detprob_flg, 1);
    }
  }
  *nbBits = nb; move16();
#ifdef DYN_RAM_CNT
  DYN_RAM_POP();
#endif
  return(detprob_flg);
}

static void getSignInfo(Word16 *avqType, const Word16 *scoef_SWB, Word16 *smdct_coef, 
                        Word32 *indexL1, Word16 *nbBitsL1, Word16 *nbBitsRestL1, 
                        Word32 *indexL2, Word16 *nbBitsL2, Word16 *nbBitsRestL2, Word16 nbBitsRest)
{
    Word16 ib, i, i8, *bandL1, nbZero;
    Word16 *ptr0, *ptr1;
    Word16 nbBits, nbBit, indexSignB, tmp;
    Word32 indexSign, mask;
    Word16 nbBitsAlloc;

#ifdef DYN_RAM_CNT
  DYN_RAM_PUSH((UWord32) (9*SIZE_Word16 + 2*SIZE_Word32 + 3 * SIZE_Ptr), "dummy");
#endif

  indexSign = 0; move32();
  bandL1 = avqType + 3;
  nbBits = nbBitsRest; move16();
  FOR (ib=0; ib<avqType[0]; ib++)
  {
      IF(nbBits > 0) 
      {
          i = bandL1[ib];
          i8 = shl (i, 3);
          ptr0 = (Word16 *)scoef_SWB +i8;
          ptr1 = smdct_coef +i8;
          nbZero = getNbZero(ptr1); 
          nbBit = s_min(nbBits, nbZero);

          nbBits = sub(nbBits, nbBit);
          indexSignB = getSign(ptr0, ptr1, nbBit);
          indexSign = L_mac0(L_shl(indexSign, nbBit), indexSignB, 1);/*  indexSign = (indexSign <<nbBit)+ indexSignB */
      }
  }
  nbBitsAlloc = sub(nbBitsRest, nbBits); /* number of bits allocated to signs */
  tmp = sub(*nbBitsRestL1, nbBitsAlloc);

  i= s_min(nbBitsAlloc, *nbBitsRestL1);
  *nbBitsL1 = add(*nbBitsL1, i);
  *indexL1 = L_shl(*indexL1, i);
  IF (tmp <= 0) 
  {
      /* all the bits in L1 allocated , -tmp bits in L2*/
      *nbBitsL2 = sub(*nbBitsL2, tmp);
      *nbBitsRestL2 = add(*nbBitsRestL2, tmp); 
      mask = L_sub(L_shr(1, tmp),1);
      *indexL2 = L_add(L_shr(*indexL2 , tmp), L_and (indexSign, mask));
  }
  /* i bits allocated  in L1 */
  *indexL1 = L_add(*indexL1, L_shl(indexSign,  s_min(tmp, 0) ));

#ifdef DYN_RAM_CNT
  DYN_RAM_POP();
#endif
  return;
}

static Word16 getSign(Word16 *ptr0, Word16 *ptr1, Word16 nbBit)
{
    Word16 indexSign;

    indexSign = 0; move16();

    WHILE(nbBit > 0)
    {
        IF (*ptr1++ == 0) 
        {
            indexSign = shl(indexSign,1);
            if(*ptr0 >=0)
            {
                indexSign = add(indexSign ,1);
            }
            nbBit = sub(nbBit,1);
        }
        ptr0++;
        
    }
    return(indexSign) ;
}
/*--------------------------------------------------------------------------*
*  Function  swbl1_encode_AVQ()                                            *
*  ~~~~~~~~~~~~~~~~~~~~~~~~~~~~                                            *
*  Main function for encoding Extension layers SWBL1 and SWBL2             *
*--------------------------------------------------------------------------*/
Word16 swbl1_encode_AVQ (
                         void*      p_AVQ_state_enc,    /* (i/o): Work space       */
                         const Word16  scoef_SWB[],     /* i:   Input SWB MDCT coefficients                  */ /* Q(scoef_SWBQ) */
                         const Word16  sEnv_BWE[],      /* i:   Input frequency envelope from SWBL0     */ /* Q(scoef_SWBQ) */
                         Word16  sratio[],              /* i: Unquantized input frequency envelope    */ /* Q(12) */
                         const Word16  index_g_5bit,    /* i:   5 bit index of frame gain from SWBL0    */
                         const Word16  cod_Mode,        /* i:   mode information from SWBL0             */
                         UWord16 *pBst_L1,              /* o:   Output bitstream for SWBL1              */
                         UWord16 *pBst_L2,              /* o:   Output bitstream for SWBL2              */
                         const Word16  layers,          /* i:   number of swb layers transmitted          */
                         const Word16  scoef_SWBQ
                         )
{

    Word16 j, ib, index_gain, ind_corr_max;
    Word16 avqType[N_SV_L1+N_SV_L2+2+N_SV+3];
    Word16 *bandL1, *bandL2, *bandZero; 
    Word16 flg_bit;
    Word16 unbits_L1, unbits_L2;
    Word16 sy_s_abs[SWB_F_WIDTH]; /* Q(12) */
    Word16 senv_BWE[N_SV]; /* Q(12) */
    Word16 Yb[SWB_F_WIDTH]; /* Q(12) */
    Word16 ord_bands[N_SV];
    Word16 scoef_SWB_AVQ[SWB_F_WIDTH], scoef_SWB_AVQ_abs[SWB_F_WIDTH];
    Word16 smdct_coef[SWB_F_WIDTH]; /* Q(12) */
    Word16 sGopt, sGopt_Q; /* Q(sGopt_Q) */ 
    Word16 sGainBWE_Q;
    Word16 sYfb_Q;
    Word16 sFenv_BWE[N_SV];
    Word16 diff_Q;
    Word16 diff2_Q;
    Word16 *ptr;
    Word16 smdct_err[SWB_F_WIDTH]; /* Q(12) */
    Word16 senv_BWE_err[N_SV]; /* Q(12) */
    Word16 svec_base[3*WIDTH_BAND]; 
    Word16 flgMode;
    Word16 detprob_flg;
    AVQ_state_enc *w_AVQ_state_enc = (AVQ_state_enc *)p_AVQ_state_enc;
    Word16 *ptr_sykr, *ptr_senv, *ptrAvqType;
    Word16 inc_cnt_detzer;
    Word16 flg_L1, flg_L2, flg_fill;
    Word16 nbBit_detprob_flg;
    Word32 indexL1, indexL2;
    Word16 nbBitsL1, nbBitsL2;
    Word16 norm_ratio[N_SV]; /* buffer for Sort() */
    UWord16 *pBst_g;
    Word16 flg_layer, offbit, flg_corr;
    Word16 nbBitsRest, nbBitsRestL1, nbBitsRestL2;
#ifdef DYN_RAM_CNT
  {
    UWord32 ssize;
    ssize = (UWord32) ((29 + (N_SV_L1 + N_SV_L2 +  N_SV) + 6 * N_SV + 3 * WIDTH_BAND + 6 * SWB_F_WIDTH) * SIZE_Word16);
    ssize += (UWord32) (2 * SIZE_Word32);
    ssize += (UWord32) (9 * SIZE_Ptr);
    DYN_RAM_PUSH(ssize, "dummy");
  }
#endif

  /*****************************/
  zero16(N_SV_L1+N_SV_L2+2+N_SV+3, avqType);
  bandL1 = avqType+3;
  bandL2 = bandL1 + N_SV_L1;
  bandZero = bandL2 + N_SV_L2+2;

  /* extract quantized frame gain */
  sGainBWE_Q = sub (14, index_g_5bit);

  /* normalize MDCT coefficients by the decoded RMS value from BWE. */
  sYfb_Q = add (scoef_SWBQ, index_g_5bit); /* Q(sYfb_Q=scoef_SWBQ+index_g_5bit) */

  /* calculate subband energy */
  loadSubbandEnergy ((Word16)cod_Mode, (Word16 *)sEnv_BWE, sFenv_BWE);

  /* compute  sratio */
  compute_sratio (sFenv_BWE, sratio, sYfb_Q);

  /* Q conversion */
  diff_Q = sub (sYfb_Q, 12);
  array_oper(SWB_F_WIDTH, diff_Q, (Word16 *)scoef_SWB, Yb, &shr);

  array_oper8(diff_Q, sFenv_BWE, senv_BWE, &shr);

  /* order spectral envelope subbands by decreasing perceptual importance */
  /* order subbands by decreasing perceptual importance  */
  Sort( sFenv_BWE, N_SV, ord_bands, norm_ratio );   /* norm_ratio used as tmp buffer only */
  flg_bit = 0;  move16();
  ptr_sykr = Yb;
  ptr_senv = senv_BWE;
  flgMode= s_or(cod_Mode, w_AVQ_state_enc->pre_cod_Mode);
  flg_layer = sub(layers, 2);
  IF(flgMode == 0)
  {
      /* write flag_bit to bitstream */
      *pBst_L1 = ITU_G192_BIT_0; move16();
      /* compute ksm and absolute value of Yb */
      flg_bit = compute_ksm(Yb, sy_s_abs, ord_bands, w_AVQ_state_enc); 
      IF (flg_bit != 0) 
      {  
          /* write flag_bit to bitstream */
          *pBst_L1 = ITU_G192_BIT_1; move16();
          /* compute MDCT error  */
          compute_mdct_err (sy_s_abs, smdct_err, (Word16 *)scoef_SWB, senv_BWE_err,
              senv_BWE);
          ptr_sykr = smdct_err;
          ptr_senv = senv_BWE_err;
      }
      pBst_L1++;
  }
  offbit = s_min(flgMode, 1);
  pBst_g = pBst_L1 + N_BITS_AVQ_L1+offbit;

  /* normalize per band, amplify for AVQ normalization & forward reorder of subbands */
  bandNormalize_Order( ptr_sykr, smdct_coef, ptr_senv, ord_bands);

  /* ***** apply algebraic vector quantization (AVQ) to MDCT coefficients * */
  encoder_SWBL1L2_AVQ( smdct_coef, &pBst_L1, &pBst_L2, layers, avqType, scoef_SWB_AVQ, &unbits_L1, &unbits_L2);

  pBst_L1 -= unbits_L1;
  pBst_L2 -= unbits_L2;

  indexL1 = 0L; move32();
  indexL2 = 0L; move32();

  nbBitsL1 = 0; move16();
  nbBitsL2 = 0; move16();
  ptrAvqType = avqType; 
  unbits_L1 = add(unbits_L1,offbit);
  
  IF( flg_bit == 0 )
  {
    /* case flg_bit = 0 */
    /* ***** detect frames with problematic zero subbands ***** */
    inc_cnt_detzer = -3; move16();
    IF( flg_layer == 0 )
    {
      inc_cnt_detzer= detectPbZeroBand_flg0(ptr_sykr, sratio, ord_bands, bandZero, avqType[2], w_AVQ_state_enc->cnt_detzer);
    }
    w_AVQ_state_enc->cnt_detzer = add(w_AVQ_state_enc->cnt_detzer , inc_cnt_detzer);
    w_AVQ_state_enc->cnt_detzer = bound(w_AVQ_state_enc->cnt_detzer , 0,  DETZER_MAX);
    IF(unbits_L1 > 0)
    {
      w_AVQ_state_enc->detzer_flg = s_min(1, w_AVQ_state_enc->cnt_detzer);
      getIndexBitstream(1, w_AVQ_state_enc->detzer_flg, &nbBitsL1, &indexL1 );
    }

    /* ***** try to find a filling of zero subbands ***** */
    IF (flg_layer == 0 )
    {
        IF(w_AVQ_state_enc->detzer_flg==0)
        {
            IF(sub( add(avqType[0],avqType[1]),N_BASE_BANDS) >= 0) 
            {
                flg_L1 = sub(unbits_L1, add(nbBitsL1,N_BITS_FILL_L1));
                flg_L2 = sub(unbits_L2, N_BITS_FILL_L2);
                flg_fill = s_max(flg_L1 , flg_L2);
                IF(flg_fill >= 0) 
                {
                    /* form base spectrum */
                    getBaseSpectrum_flg0(avqType, scoef_SWB_AVQ, svec_base);
                    /* try to fill zero band */
                    flg_corr = 1; move16();
                    /* reconstruct the zero subband 1 */
                    IF(flg_L1 >= 0)
                    {
                        ind_corr_max = fillZeroBands_flg0(0, avqType, smdct_coef, scoef_SWB_AVQ,
                            svec_base);
                        flg_corr = s_xor(ind_corr_max, CORR_RANGE_L1);
                        getIndexBitstream( N_BITS_FILL_L1, ind_corr_max, &nbBitsL1, &indexL1);
                    }
                    /* reconstruct the zero subband 2 */
                    flg_L2 = s_min(flg_L2, sub(avqType[1], 1));
                    IF(flg_L2 >= 0)
                    {
                        IF(flg_corr = 0) 
                        {
                            j = bandZero[0]; move16();
                            bandZero[0]  = bandZero[1]; move16();
                            bandZero[1] = j; move16();
                        }
                        ind_corr_max = fillZeroBands_flg0(QCOEF, avqType, smdct_coef, scoef_SWB_AVQ, 
                            svec_base);
                        getIndexBitstream( N_BITS_FILL_L2, ind_corr_max, &nbBitsL2, &indexL2);
                    }
                } /* flg_fill >= 0 */
            } /*avqType[0]+ avqType[1])>=N_BASE_BANDS*/
        } /* w_AVQ_state_enc->detzer_flg=0 */
    } /* flg_layer = 0 */

    /* backward reordering */
    ptr = avqType + 3+ N_SV_L1;
    sortIncrease(avqType[1], avqType[1], ptr, ptr);
    bwdReorder (ptr_senv, scoef_SWB_AVQ, smdct_coef, ord_bands, avqType);

    /* compute global gain adjustment */
    sGopt = ggain_adj ( bandL1, avqType[0], Yb, smdct_coef, index_g_5bit, &sGopt_Q);

    /* Embedded coding of the adjusted gain in log2 domain */
    index_gain = cod_emb_fgain (index_g_5bit, &sGopt);
    IF (flg_layer == 0 )
    {
      /** calculate locally decoded MDCT coefs.---------------*/               
      /* for zero subbands, keep MDCT coeficients from the BWE SWBL0 */
      diff_Q = sub (sub (sGainBWE_Q, 3), scoef_SWBQ);
      diff2_Q = sub (sub (12, sGainBWE_Q), scoef_SWBQ);      
      FOR (ib=0; ib<avqType[0]; ib++)
      {
        ptr = smdct_coef + shl(bandL1[ib],3);

        /* apply adjusted global gain to AVQ decoded MDCT coefs */
        FOR (j=0; j<WIDTH_BAND; j++)
        {
          /* coef_SWB_AVQ[j] *= fGopt; */
          ptr[j] = shr (mult_r (ptr[j], sGopt), diff_Q); /* Q(scoef_SWBQ):(12+sGainBWE_Q+1-16)-((sGainBWE_Q-3)-scoef_SWBQ) */
          move16();
        }
      }
      FOR (ib=0; ib<avqType[1]; ib++)
      {
        ptr = smdct_coef + shl(bandL2[ib],3);

        array_oper8(diff2_Q, ptr, ptr, &shr);
      }
      /*-calculate gradient and modify locally decoded MDCT coefs. */   
      encoder_coef_SWB_AVQ_adj(scoef_SWB, bandL1, avqType[0], smdct_coef, 
                               &indexL1, &nbBitsL1, sub(unbits_L1,nbBitsL1) );
      encoder_coef_SWB_AVQ_adj(scoef_SWB, bandL2, avqType[1],smdct_coef, 
                               &indexL2, &nbBitsL2, sub(unbits_L2, nbBitsL2));
      nbBitsRestL1 = sub(unbits_L1,nbBitsL1);
      nbBitsRestL2 = sub(unbits_L2,nbBitsL2);
    }
  }
  ELSE 
  {
    /* case flg_bit = 1 */
    /* backward reordering */
    bwdReorder (ptr_senv, scoef_SWB_AVQ, smdct_coef, ord_bands, avqType);

    /* compute global gain adjustment */
    /* ------------------------------ */
    globalGainAdj (avqType, smdct_coef, scoef_SWB_AVQ_abs, senv_BWE);

    sGopt = ggain_adj (bandL1, avqType[0], sy_s_abs, scoef_SWB_AVQ_abs, index_g_5bit, &sGopt_Q);

    IF (flg_layer == 0 )
    {
      detprob_flg = detectPbZeroBand_flg1(avqType[2], bandZero, unbits_L1, sratio, &nbBit_detprob_flg );
      getIndexBitstream(nbBit_detprob_flg, detprob_flg, &nbBitsL1, &indexL1 );
    }
    w_AVQ_state_enc->cnt_detzer = sub(w_AVQ_state_enc->cnt_detzer, 3);
    w_AVQ_state_enc->cnt_detzer = bound(w_AVQ_state_enc->cnt_detzer , 0,  DETZER_MAX);


    IF (flg_layer == 0 )
    {
        flg_L1 = sub(unbits_L1, add(nbBitsL1, N_BITS_FILL_L1));
        IF( sub(add(avqType[0],avqType[1]), N_BASE_BANDS) >= 0)
        {
            flg_L2 = sub(unbits_L2, N_BITS_FILL_L2);
            flg_fill = s_max(flg_L1 , flg_L2);
            IF(flg_fill >= 0) 
            {
                getBaseSpectrum_flg1 (avqType, senv_BWE, (Word16 *)sEnv_BWE, svec_base, scoef_SWB_AVQ_abs, smdct_coef, (Word16 *)scoef_SWB, scoef_SWBQ);
                sortIncrease(avqType[2], s_min(2, avqType[2]), bandZero, bandZero); 
                flg_corr = 1; move16();
                /* reconstruct the zero subband 1 */
                IF(flg_L1 >= 0)
                {
                    ind_corr_max = fillZeroBands_flg1(avqType, bandZero, (Word16 *)scoef_SWB, scoef_SWB_AVQ_abs, senv_BWE, svec_base);
                    flg_corr = s_xor(ind_corr_max, CORR_RANGE_L1);
                    getIndexBitstream( N_BITS_FILL_L1, ind_corr_max, &nbBitsL1, &indexL1);
                }
                /* reconstruct the zero subband 2 */
                IF(flg_L2 >= 0)
                {
                    IF(flg_corr == 0)
                    {
                        IF( sub(avqType[2], 1) > 0 )
                        {
                            j = bandZero[0]; move16();
                            bandZero[0]  = bandZero[1]; move16();
                            bandZero[1] = j; move16();
                        }
                    }
                    ind_corr_max = fillZeroBands_flg1(avqType, bandZero, (Word16 *)scoef_SWB, scoef_SWB_AVQ_abs, senv_BWE, svec_base);
                    getIndexBitstream( N_BITS_FILL_L2, ind_corr_max, &nbBitsL2, &indexL2);
                }
            } /* (flg_fill >= 0)  */
        } /*avqType[0]+avqType[1] >=N_BASE_BANDS*/
        ELSE
        {
            IF(flg_L1 >= 0)
            {
                getIndexBitstream(N_BITS_FILL_L1, 0, &nbBitsL1, &indexL1);
            }
        }
    } /* layers = 2*/

    /*-- send sign information----------------------------*/  
    /* set pointer adress */
    IF (sub (layers, 1) == 0)
    {
      nbBitsL1 = unbits_L1; move16();
      nbBitsL1 = s_min(N_BITS_FILL_L1+2, nbBitsL1);
      nbBitsL2 = 0; move16();
    }

    nbBitsRestL1 = sub(unbits_L1,nbBitsL1);
    nbBitsRestL2 = sub(unbits_L2,nbBitsL2);
    nbBitsRest = add(nbBitsRestL1, nbBitsRestL2);
    IF ( L_mult(avqType[0], nbBitsRest) > 0) 
    { 
      /* allocate sign information */
      sortIncrease(avqType[0], avqType[0], bandL1, bandL1); 
      getSignInfo( avqType, scoef_SWB, smdct_coef, &indexL1, &nbBitsL1, &nbBitsRestL1, &indexL2, &nbBitsL2, &nbBitsRestL2, nbBitsRest);
    }
    /* Embedded coding of the adjusted gain in log2 domain */
    index_gain = cod_emb_fgain (index_g_5bit, &sGopt);
  }
  PushBitLong( indexL1, &pBst_L1, nbBitsL1);
  PushBit( index_gain, &pBst_g, N_BITS_GAIN_SWBL1);
  PushBitLong( indexL2, &pBst_L2, nbBitsL2);
  pBst_L2 += nbBitsRestL2;
  w_AVQ_state_enc->pre_cod_Mode = cod_Mode;   move16();
  /*****************************/
#ifdef DYN_RAM_CNT
  DYN_RAM_POP();
#endif
  /*****************************/
  return 0;
}

/*-----------------------------------------------------------------*
*   Function  ggain_adj                                           *
*                                                                 *
*   Compute global gain adjustment                                *
*-----------------------------------------------------------------*/
static Word16 ggain_adj(
                        Word16 *bandL1, Word16 nbBandL1, Word16 *sx,
                        Word16 *sqx, Word16 index_g_5bit, Word16 *sGopt_Q)
{
  Word16 ib, j;
  Word16 *p_x, *p_qx, sGaf;
  Word16 sden, snum, sden_Q, snum_Q, diff_Q;
  Word32 lnum, lden, lnumtmp, ldentmp;

  /*****************************/
#ifdef DYN_RAM_CNT
  DYN_RAM_PUSH((UWord32) (2 * SIZE_Ptr + 8 * SIZE_Word16 + 4 * SIZE_Word32), "dummy");
#endif
  /*****************************/

  lnum = 0L; move32 ();
  lden = 1L; move32 ();
  FOR (ib=0; ib<nbBandL1; ib++)
  {
      j = shl(bandL1[ib],3);
      p_x = sx + j;
      p_qx = sqx + j;

      lnumtmp = L_shr (L_mult0 (p_x[0], p_qx[0]), 3);
      ldentmp = L_shr (L_mult0 (p_qx[0], p_qx[0]), 3);
      FOR (j=1; j<WIDTH_BAND; j++)
      {
          lnumtmp = L_add (lnumtmp, L_shr (L_mult0 (p_x[j], p_qx[j]), 3));
          ldentmp = L_add (ldentmp, L_shr (L_mult0 (p_qx[j], p_qx[j]), 3)); 
      }
      lnum = L_add (lnum, L_shr (lnumtmp, 1)); /* Q(2*sx_Q-4) */
      lden = L_add (lden, L_shr (ldentmp, 1)); /* Q(2*sx_Q-4) */
  }

  /* Q-value, "2*sx_Q-4", will be cancelled at mult_r(). */
  sden = extract_h(norm_l_L_shl(&sden_Q, lden));
  sden_Q = sub (sden_Q, 16); /* Q(sden_Q) */
  sden = div_s (16384, sden); /* Q(-sden_Q+29):15+14-(sden_Q) */
  snum = extract_h(norm_l_L_shl(&snum_Q, lnum));
  snum_Q = sub (snum_Q, 16); /* Q(snum_Q) */

  sGaf = mult_r (snum, sden); /* Q(snum_Q-sden_Q+14):snum_Q+(-sden_Q+29)+1-16 */
  *sGopt_Q = add (sub (snum_Q, sden_Q), 14);
  if (sGaf == 0)
  {
    *sGopt_Q = 14; move16 ();
  }

  diff_Q = sub(*sGopt_Q, 14);
  *sGopt_Q = 14; move16 ();
  if (diff_Q != 0)
  {
    sGaf = shr (sGaf, diff_Q); /* Q14 */
  }
  /*****************************/
#ifdef DYN_RAM_CNT
  DYN_RAM_POP();
#endif
  /*****************************/

  /* Obtain adjusted gain */
  *sGopt_Q = sub (14, index_g_5bit); /* Q(sGopt_Q=14-index_g_5bit) */

  return(sGaf);
}

static Word16 cod_emb_fgain (Word16 index_g_5bit,
                             Word16 *sgopt)
{
  Word16 i, min_index_frac, sgopt_q;
  Word32 ldtmp, lmin_dist;
  /*****************************/
#ifdef DYN_RAM_CNT
  {
    UWord32 ssize;
    ssize = (UWord32) (3 * SIZE_Word16);
    ssize += (UWord32) (2 * SIZE_Word32);
    DYN_RAM_PUSH(ssize, "dummy");
  }
#endif
  /*****************************/

  /* Additional 3bit search of frame gain */
  IF (index_g_5bit == 0)
  {
    min_index_frac = minDiff0Array16(5, *sgopt, (Word16 *)sg0, &lmin_dist); 
    sgopt_q = sg0[min_index_frac];
    i = minDiff0Array16(3, *sgopt, (Word16 *)&sgain_frac[5], &ldtmp); 
    IF(L_sub (ldtmp, lmin_dist) <= 0)
    {
      min_index_frac = add(i,5);
      sgopt_q = sgain_frac[min_index_frac];
    }
  }
  ELSE
  {
    min_index_frac = minDiff0Array16(8, *sgopt, (Word16 *)sgain_frac, &lmin_dist); 
    sgopt_q = sgain_frac[min_index_frac];
  }
  *sgopt = sgopt_q ;

  /*****************************/
#ifdef DYN_RAM_CNT
  DYN_RAM_POP();
#endif
  /*****************************/

  return(min_index_frac);
}

static Word16 minDiff0Array16( Word16 n, Word16 x, Word16 *y, Word32 *Lmin_dist)
{
  Word16 i, min_index, tmp;
  Word32 ldtmp, lmin_dist;
  /*****************************/
#ifdef DYN_RAM_CNT
  {
    UWord32 ssize;
    ssize = (UWord32) (3 * SIZE_Word16);
    ssize += (UWord32) (2 * SIZE_Word32);
    DYN_RAM_PUSH(ssize, "dummy");
  }
#endif
  /*****************************/    
  /* Additional 3bit search of frame gain */
  min_index = 0; move16 ();

  tmp = sub(x, y[0]);
  lmin_dist = L_mult0 (tmp, tmp); /* Q(2*sgopt_Q):sgopt_Q+sgopt_Q */
  FOR (i=1; i<n; i++) 
  {
    tmp = sub(x, y[i]);
    ldtmp = L_mult0 (tmp, tmp); /* Q(2*sgopt_Q):sgopt_Q+sgopt_Q */
    if(L_sub (ldtmp, lmin_dist) <= 0)
    {
      min_index = i; move16 ();
    }
    lmin_dist = L_min(lmin_dist, ldtmp);
  }

  *Lmin_dist = lmin_dist; move32();
  /*****************************/
#ifdef DYN_RAM_CNT
  DYN_RAM_POP();
#endif
  /*****************************/
  return(min_index);
}

/*-----------------------------------------------------------------*
*   Funtion  encoder_coef_SWB_AVQ_adj                             *
*            ~~~~~~~~~~~~~~~~~~~~~~~~                             *
*   calculate gradient to modify locally decoded MDCT coefs.       *
*-----------------------------------------------------------------*/
static void encoder_coef_SWB_AVQ_adj( 
                                     const Word16 *scoef_SWB,  /* i:  MDCT coefficients to encode */
                                     Word16  *bandL,           /* i:  input vector signalising AVQ type  (0, L1, L2) */
                                     Word16  nbBand,           /* i:  nb bands of AVQ type  (0, L1, L2) */
                                     Word16  *scoef_SWB_AVQ,   /* i/o:  locally decoded MDCT coefs. */
                                     Word32  *indexL,
                                     Word16  *nbBitsL,
                                     Word16   unbits
                                     )                        
{
  Word16 i, ig, ib, n, nbGrad2, nbGrad1, max_idx;
  Word16 idx, index, bit_alloc;
  Word16 *ptrBand, *ptr0, *ptr1;
  /*****************************/
#ifdef DYN_RAM_CNT
  DYN_RAM_PUSH((UWord32) (10 * SIZE_Word16 + 3 * SIZE_Ptr), "dummy");
#endif
  /*****************************/
  IF(unbits > 0) 
  {
    /* calculate the number of bands with  2 bits gradient */
    n = sub(unbits, nbBand);
    nbGrad2 = s_min(nbBand, n);
    nbGrad2 = s_max(nbGrad2 , 0);
    nbGrad1 = sub(s_min(nbBand, unbits),nbGrad2); 

    /* calculate gradient */
    /* ------------------ */
    bit_alloc = shl(nbGrad2, 1);
    index = 0; move16();
    n = nbGrad2; move16();
    max_idx = 3; move16();
    ptrBand = bandL;
    FOR(ig = 2; ig>0; ig--)
    {
        FOR (ib=0; ib<n; ib++)
        {
            /* calculate gradient of each vector */
            i= shl(*ptrBand++,3);
            ptr0 = (Word16*)scoef_SWB + i;
            ptr1 = scoef_SWB_AVQ + i;

            idx = compute_errGradNormL1(ptr0, ptr1, (Word16*)sgrad, max_idx); move16();
            index = shl(index, ig);
            index = add(index, idx);
        }
        n = nbGrad1;
        max_idx = sub(max_idx,2);
    }
    bit_alloc = add(bit_alloc, nbGrad1);
    getIndexBitstream(bit_alloc, index, nbBitsL, indexL);
  }
  /*****************************/
#ifdef DYN_RAM_CNT
  DYN_RAM_POP();
#endif
  /*****************************/ 

  return;
}

static Word16 compute_errGradNormL1(Word16 *x, Word16 *xq, Word16 *sgrad, Word16 max_idx)
{
  Word32 min_err, err;
  Word16 tmp, k, j, idx;
  Word16 *ptr0, *ptr1, *ptr2;
#ifdef DYN_RAM_CNT
  DYN_RAM_PUSH((UWord32)(4 * SIZE_Word16 + 3 * SIZE_Ptr + 2 * SIZE_Word32), "dummy");
#endif

  ptr0 = x;
  ptr1 = xq;
  /* calculate error between Yfb and coef_SWB_AVQ */
  tmp = sub(*ptr0++, *ptr1++);
  min_err = L_deposit_l(abs_s(tmp));
  FOR (j=1; j<WIDTH_BAND; j++)
  {
    tmp = sub(*ptr0++, *ptr1++);
    min_err = L_mac0( min_err, abs_s(tmp), 1);
  }

  /* compare errors */
  idx = 0; move16();
  ptr2 = sgrad;
  FOR (k=0; k<max_idx; k++)
  {
    ptr0 -= WIDTH_BAND;
    ptr1 -= WIDTH_BAND;

    err = L_abs( L_msu0( L_deposit_l( shl( mult(*ptr1++, *ptr2++), 1) ), *ptr0++, 1 ) );
    FOR (j=1; j<WIDTH_BAND; j++)
    {
      err = L_add( err, L_abs( L_msu0( L_deposit_l( shl( mult(*ptr1++, *ptr2++), 1) ), *ptr0++, 1) ));
    }

    if (L_sub(min_err, err) > 0) 
    { 
      idx = add(k, 1); 
    } 
    min_err = L_min(err, min_err);
  }  
#ifdef DYN_RAM_CNT
  DYN_RAM_POP();
#endif
  return(idx);
}

/* normalize per band, amplify for AVQ normalization & forward reorder of subbands */
static void bandNormalize_Order(const Word16 *sykr, Word16 *smdct_coef, const Word16 *senv_BWE, 
                                const Word16 *ord_bands)
{
  Word16 *ptr0;
  const Word16 *ptr;

  Word16 i, j;
  Word16 exp_den, den, exp_num, iGain16;
#ifdef DYN_RAM_CNT
  DYN_RAM_PUSH((UWord32) (6 * SIZE_Word16 + 2 * SIZE_Ptr), "dummy");
#endif

  ptr0 =  smdct_coef; 

  FOR( i=0; i<N_SV; i++ )
  {
    /* initialization */
    ptr  = sykr+ shl(ord_bands[i],3);
    IF( senv_BWE[ord_bands[i]] > 0)
    {
      /* Invert Gain */
      exp_den = norm_s(senv_BWE[ord_bands[i]]);
      den = shl(senv_BWE[ord_bands[i]], exp_den);
      iGain16 = div_s(INV_CNST_WEAK_FX2, den);
      exp_num = sub(4, exp_den); /* normalized smdct_coef in Q0 */
      FOR( j=0; j<WIDTH_BAND; j++ )
      {
        *ptr0++ = round_fx_L_shr_L_mult(*ptr++, iGain16, exp_num);
        move16();
      }
    }
    ELSE
    {
      zero16_8(ptr0 );
      ptr0 += WIDTH_BAND;
    }
  }

#ifdef DYN_RAM_CNT
  DYN_RAM_POP();
#endif
  return;
}

/* ***** detect frames with problematic zero subbands ***** */
static Word16 detectPbZeroBand_flg0(const Word16 *sykr, const Word16 *sratio_fEnv, 
                                    const Word16 *ord_bands, Word16 *bandZero, Word16 nbBand0, Word16 cnt_detzer)
{
  Word16 smax_ratio, detzer_flg1, detzer_flg2, inc_cnt_detzer;
  Word16 ib, i, j, k;
  Word32 Lmax_band ,  L_tmp, L_en;
  Word16 smean_band, smax_band; 
  const Word16 *ptr;
#ifdef DYN_RAM_CNT
  DYN_RAM_PUSH((UWord32) (10 * SIZE_Word16 + 3 * SIZE_Word32 + SIZE_Ptr), "dummy");
#endif

  detzer_flg1 = 0;        move16();
  smax_ratio = 0;         move16();
  FOR( ib=0; ib<nbBand0; ib++ )
  {
    i = bandZero[ib];    move16();
    k = ord_bands[i];   move16();
    smax_ratio = s_max(smax_ratio, sratio_fEnv[k]);
  }
  if (sub(smax_ratio, 16384 /*(4 in Q12)*/) > 0 )
  {
    detzer_flg1 = 1;         move16();
  }

  detzer_flg2 = 0;        move16();
  FOR( ib=0; ib<nbBand0; ib++ )
  {
    IF(sub(detzer_flg2, 2) != 0 ) 
    {
      i = bandZero[ib];    move16();
      k = ord_bands[i];   move16();
      Lmax_band  = 0;       move32();
      L_tmp      = 0;       move32();
      ptr = sykr +shl(k,3);
      FOR( j = 0; j<WIDTH_BAND; j++ )
      {
        L_en = L_mult0(ptr[j], ptr[j]);
        L_tmp = L_add(L_tmp, L_en);
        Lmax_band = L_max(L_en, Lmax_band);
      }
      /*mean_band /= WIDTH_BAND;*/
      smean_band = round_fx(L_shr(L_tmp, 3));
      smax_band = round_fx(Lmax_band);
      IF (sub(mult_r(smax_band, 5461), smean_band) > 0) 
      {               
        detzer_flg2 = 2; move16();            /* could be: leave the loop */
      }
      /* max_band > 4.0*mean_band )*/
      ELSE
      {
        IF( sub(mult_r(smax_band, 8192), smean_band) > 0)  
        {               
          detzer_flg2 = 1; move16();
        }
      }
    }
  }
  if(sub(nbBand0,5)<0) 
  {
    detzer_flg2 = s_and(detzer_flg2, 2); 
  }
  inc_cnt_detzer = DETZER_MAX; move16();
  IF( detzer_flg1 == 0 ) 
  {
    inc_cnt_detzer = sub(detzer_flg2, 2);
    IF( (cnt_detzer > 0) )
    {
      if( inc_cnt_detzer == 0) {
        inc_cnt_detzer = add(inc_cnt_detzer, 3);
      }
    }
  }
#ifdef DYN_RAM_CNT
  DYN_RAM_POP();
#endif
  return(inc_cnt_detzer);
}

/* ***** try to find a filling of zero subbands ***** */
static void getBaseSpectrum_flg1 (Word16 *avqType, Word16 *senv_BWE, Word16 *sEnv_BWE , Word16 *svec_base, 
                                  Word16 *scoef_SWB_AVQ_abs, Word16 *scoef_SWB_AVQ,
                                  Word16 *scoef_SWB, Word16 scoef_SWBQ)
{
  Word16 ib, i8, i, j;
  Word16 *ptr0, *ptr1, *ptr2, *ptr;
  Word16 exp_num0, exp_num1, exp_num, Gain0, Gain1, stmp;
  Word16 bandLoc[N_SV_L1+N_SV_L2];
#ifdef DYN_RAM_CNT
  DYN_RAM_PUSH((UWord32) ((10 + N_SV_L1 + N_SV_L2) * SIZE_Word16 + 4 * SIZE_Ptr), "dummy");
#endif
  i = avqType[0];move16();
  mov16(i, &avqType[3], bandLoc);
  mov16(avqType[1], &avqType[6], &bandLoc[i]);
  i = add(i,avqType[1]);
  sortIncrease(i, 3, bandLoc, bandLoc);
  ptr = svec_base;
  exp_num = sub(14, scoef_SWBQ);
  FOR(ib=0; ib<3; ib++) 
  {
    i = bandLoc[ib]; move16();
    i8 = shl(i, 3);
    ptr0 = scoef_SWB_AVQ_abs + i8;
    ptr2 = scoef_SWB_AVQ + i8;
    ptr1 = scoef_SWB + i8;
    /* Compute Gains 0 & 1 */
    Gain0 = invEnv_BWE(sEnv_BWE[i], exp_num, &exp_num0);
    Gain1 = invEnv_BWE(senv_BWE[i], 2, &exp_num1);
    FOR( j=0; j<WIDTH_BAND; j++ )
    {
      IF(ptr2[j] == 0) 
      {
        stmp = round_fx_L_shr_L_mult(ptr0[j], Gain0, exp_num0);
        if(ptr1[j] < 0) stmp = negate(stmp);      
      }
      ELSE 
      {
        stmp = round_fx_L_shr_L_mult(ptr0[j], Gain1, exp_num1);
        if(ptr2[j] < 0) stmp = negate(stmp);      
      }
      ptr[j] = stmp;    move16();
    }
    ptr += WIDTH_BAND;
  }
#ifdef DYN_RAM_CNT
  DYN_RAM_POP();
#endif
  return;
}

static void getVecToFill_flg1(Word16 senv_BWE, Word16 *scoef_SWB, Word16 *vecToFill)
{
  Word16 j;
  Word16 exp_num, Gain;
#ifdef DYN_RAM_CNT
  DYN_RAM_PUSH((UWord32) (3 * SIZE_Word16), "dummy");
#endif
  /* inverse Fenv_BWE[i] */
  Gain = invEnv_BWE(senv_BWE, 2, &exp_num);
  FOR( j=0; j<WIDTH_BAND; j++ )
  {
    /*vec_base[tmp16+j] = coef_SWB[i8+j]/Fenv_BWE[i];*/
    vecToFill[j] = round_fx_L_shr_L_mult(scoef_SWB[j], Gain, exp_num);
    move16();
  }
#ifdef DYN_RAM_CNT
  DYN_RAM_POP();
#endif
  return;
}

static Word16 invEnv_BWE(Word16 sEnv, Word16 expx, Word16 *exp_num)
{
  Word16 tmp16, den, iGain16, exp_den;
#ifdef DYN_RAM_CNT
  DYN_RAM_PUSH((UWord32) (4 * SIZE_Word16), "dummy");
#endif

  tmp16 = s_max(1, sEnv);  /* To ensure a non-zero value */
  exp_den = norm_s(tmp16);
  den = shl(tmp16, exp_den);
  iGain16 = div_s(16384, den);
  *exp_num = sub(expx,exp_den);  

#ifdef DYN_RAM_CNT
  DYN_RAM_POP();
#endif
  return (iGain16);
}

/* ***** try to find a filling of zero subbands ***** */
static Word16 fillZeroBands_flg0(Word16 Qval, Word16 *avqType, 
                               Word16 *smdct_coef_nq, Word16 *smdct_coef_avq, Word16 *svec_base)
{
  Word16 i8, j, nbBand2, ind_corr_max;
  Word16 Gain16 ; 
  Word16 *ptrBaseSpectrum, *vecToFill, *ptrBand;
#ifdef DYN_RAM_CNT
  DYN_RAM_PUSH((UWord32) (5 * SIZE_Word16 + 3 * SIZE_Ptr), "dummy");
#endif
  ptrBand = &avqType[3+N_SV_L1+N_SV_L2+2];
  i8 = shl(ptrBand[0], 3);
  vecToFill = smdct_coef_nq + i8;


  /* ===== compute correlations for zero subband and reconstruction ===== */
  ind_corr_max= getParamFillBand(svec_base, vecToFill , Qval, &Gain16 );
  
  IF(s_xor(ind_corr_max,CORR_RANGE_L1) !=  0) 
  {
    ptrBaseSpectrum = svec_base + ind_corr_max;
    vecToFill = smdct_coef_avq + i8;
    IF( sub(Gain16,32767)< 0 )
    {
      FOR( j=0; j<WIDTH_BAND; j++ )
      {
        vecToFill[j] = mult_r(shl(ptrBaseSpectrum[j],Qval), Gain16);     move16();
      }
    }
    ELSE
    {
      array_oper(WIDTH_BAND, Qval, ptrBaseSpectrum, vecToFill, &shl);
    }
    nbBand2 = avqType[1]; move16();
    ptrBand[sub(nbBand2,N_SV_L2+2)] = ptrBand[0]; move16();
    avqType[1] = add(avqType[1], 1); move16();
    avqType[2] = sub(avqType[2], 1); 
    mov16(avqType[2], &ptrBand[1], ptrBand);
  }

#ifdef DYN_RAM_CNT
  DYN_RAM_POP();
#endif
  return (ind_corr_max);
}

static Word16 fillZeroBands_flg1(Word16 *avqType, Word16 *iZero, 
    Word16 *scoef_SWB, Word16 *scoef_SWB_AVQ_abs, Word16 *senv_BWE, Word16 *svec_base)
{
  Word16 i8, j;
  Word16 Gain16, ind_corr_max; 
  Word16 *ptr0, *ptr2, *ptrBand;
  Word16 nbBand2;
#ifdef DYN_RAM_CNT
  DYN_RAM_PUSH((UWord32) (5 * SIZE_Word16 + 3 * SIZE_Ptr), "dummy");
#endif
  ptrBand= &avqType[3+N_SV_L1+N_SV_L2+2];
  i8 = shl(*iZero,3);
  ptr0 = scoef_SWB_AVQ_abs + i8;
  getVecToFill_flg1( senv_BWE[*iZero], scoef_SWB+i8, ptr0);
  /*  compute correlations for first zero subband and reconstruction ===*/
  ind_corr_max = getParamFillBand(svec_base, ptr0, 0, &Gain16);
  IF(s_xor(ind_corr_max,CORR_RANGE_L1) !=0 ) 
  {
    ptr2 = svec_base + ind_corr_max;
    IF( sub(Gain16,32767) < 0 )
    {
      Gain16 = mult_r(Gain16,senv_BWE[*iZero]);
    }
    ELSE 
    {
      Gain16 = senv_BWE[*iZero];   move16();
    }
    FOR( j=0; j<WIDTH_BAND; j++ )
    {
      ptr0[j] = round_fx_L_shl_L_mult(ptr2[j],Gain16,3);
      move16();
    }
    nbBand2 = avqType[1]; move16();
    ptrBand[sub(nbBand2,N_SV_L2+2)] = *iZero; move16();
    avqType[1] = add(avqType[1], 1); move16();

  }

  *iZero = ptrBand[1]; move16();

#ifdef DYN_RAM_CNT
  DYN_RAM_POP();
#endif

  return (ind_corr_max);
}

static void globalGainAdj (Word16 *avqType, Word16 *scoef_SWB_AVQ, 
                           Word16 *scoef_SWB_AVQ_abs, Word16 *senv_BWE)
{
  Word16 iL, ib, j, i8; 
  Word16 *ptrNbBand, *ptrBand;
  Word16 *ptr0, *ptr1, cnt, stmp, senv_tmp, norm_ltmp;
  Word32 lbuff_coef_pow, ltmp, lbuff_Fenv_BWE;
#ifdef DYN_RAM_CNT
  DYN_RAM_PUSH((UWord32) (8 * SIZE_Word16 + 3 * SIZE_Word32 + 4 * SIZE_Ptr), "dummy");
#endif

  ptrNbBand = avqType;
  ptrBand = avqType + 3;
  FOR(iL=0; iL<2; iL++) 
  {
    FOR(ib= 0; ib<*ptrNbBand; ib++)
    {
      i8 = shl(ptrBand[ib], 3);
      ptr0 = scoef_SWB_AVQ + i8;
      ptr1 = scoef_SWB_AVQ_abs + i8;
      /* calculate abs. value of coef_SWB_AVQ */
      cnt = 0; move16 ();
      senv_tmp = shr (senv_BWE[ptrBand[ib]], 1);
      lbuff_coef_pow = 0L; move32();
      FOR (j=0; j<WIDTH_BAND; j++)
      {
        IF( *ptr0 !=0) 
        {
          stmp = abs_s(*ptr0);
          cnt = add(cnt, 1);
          *ptr1 = add (stmp, senv_tmp); move16 (); /* Q12 */
          stmp = shr (*ptr1, 3); /* Q9:12-3 */
          lbuff_coef_pow = L_mac (lbuff_coef_pow, stmp, stmp); /* Q19:9+9+1 */
        }
        ptr0++;
        ptr1++;
      }
      /* calculate buff_Fenv_BWE */
      ltmp = L_mult (senv_BWE[ptrBand[ib]], senv_BWE[ptrBand[ib]]);/* Q22:12+12+1-3 */

      ltmp = L_shr (ltmp, 3); /* Q19:22-3 */
      ltmp = L_sub (ltmp, lbuff_coef_pow); /* Q19 */
      ltmp = norm_l_L_shl(&norm_ltmp, ltmp);
      lbuff_Fenv_BWE = 0; move32();
      if (ltmp > 0) lbuff_Fenv_BWE = L_shr (L_mult (round_fx (ltmp), dentbl[cnt]), norm_ltmp); /* Q19:3+norm+15+1-norm */
      SqrtI31 (lbuff_Fenv_BWE, &lbuff_Fenv_BWE); /* Q25:31-(31-19)/2 */
      /* calculate abs. value of coef_SWB_AVQ */
      stmp = round_fx (L_shl (lbuff_Fenv_BWE, 3)); /*Q12:25+3-16*/
      ptr0 -= WIDTH_BAND;
      ptr1 -= WIDTH_BAND;
      FOR (j=0; j<WIDTH_BAND; j++)
      {
        if (*ptr0== 0)
        {
          *ptr1 = stmp; move16 ();
        }
        ptr0++;
        ptr1++;
      }
    }
    ptrNbBand++;
    ptrBand += 3;
  }
  ptrBand += 3;
  FOR(ib= 0; ib<*ptrNbBand; ib++)
  {
    ptr1 = scoef_SWB_AVQ_abs + shl(ptrBand[ib],3);

    zero16_8(ptr1);
  }

#ifdef DYN_RAM_CNT
  DYN_RAM_POP();
#endif
  return;
}

static void getIndexBitstream( Word16 nbBit, Word16 val, Word16 *nbBitCum, Word32 *index)
{
  *nbBitCum = add(*nbBitCum,nbBit);
  *index = L_shl(*index,  nbBit);

  *index = L_mac0(*index, val, 1);

  return;
}
