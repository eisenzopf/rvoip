/* ITU G.722 3rd Edition (2012-09) */

/*--------------------------------------------------------------------------
 ITU-T G.722 Annex C (ex G.722-SWB-Float) Source Code
 Software Release 1.01 (2012-07)
 (C) 2012 France Telecom, Huawei Technologies, VoiceAge Corp., NTT.
--------------------------------------------------------------------------*/

#include "bit_op.h"
#include "bwe.h"
#include "avq.h"
#include "rom.h"

#include <math.h>

/*------------------------------------------------------------------------*
* Prototypes
*------------------------------------------------------------------------*/    
static void s_getIndexBitstream( Short nbBit, Short val, Short *nbBitCum, long *index);
static void s_sortIncrease(
                         Short n,         /* i  : array dimension */
                         Short nbMin,    /* i:  number of minima to be sorted */
                         Short *xin,    /* i  : arrray to be sorted */ 
                         Short *xout    /* o  : sorted array  */
                         );

static void compute_sratio (Float *sEnv_BWE, Float *sratio);
static Short compute_ksm(Float *fYb, Float *fy_s_abs, Short *sord_b, 
                          AVQ_state_enc *w_AVQ_state_enc); 
static void compute_mdct_err (Float *fy_s_abs, Float *fmdct_err, 
                              Float *fcoef_SWB, Short *mdct_sign, Float *fenv_BWE_err, Float *fenv_BWE);
static Short detectPbZeroBand_flg0(const Float *fykr, const Float *fratio_fEnv, 
                                    const Short *ord_bands, Short *bandZero, Short nbBand0, Short cnt_detzer);
static Short detectPbZeroBand_flg1(Short nbBandZero, Short *bandZero, 
                                    Short unbits_L1, Float *fratio, Short *nbBits);
static void encoder_SWBL1L2_AVQ(const float *fmdct_coef, 
                                unsigned short **pBst_L1,  unsigned short **pBst_L2, const Short layers, Short *avqType, float *fcoef_SWB_AVQ,  Short *unbits_L1, Short *unbits_L2);
static Float ggain_adj( Short  *bandL1, Short nbBandL1, Float *f_sx, Float *f_sqx, Short index_g_5bit);
static Short cod_emb_fgain (Short index_g_5bit, Float *fGopt);
static Short minDiff0Array16( Short n, Float x, Float *y, Float *Lmin_dist);
static void encoder_coef_SWB_AVQ_adj(const Float fcoef_SWB[], Short bandL[], Short nbBand, Float fcoef_SWB_AVQ[], long *indexL_, Short *nbBitsL_, Short nbBitsTot);
static Short compute_errGradNormL1(Float *x, Float *xq, Float *fgrad, Short max_idx);
static void bandNormalize_Order( const Float *fykr, Float *fmdct_coef, const Float *fenv_BWE, const Short *ord_bands);
static void bandNormalize_Order_flt( const Float *fykr, Float *fmdct_coef, const Float *fenv_BWE, const Short *ord_bands);
static void bwdReorder (const Float *fenv_BWE, Float *fmdct_coef, Float *fcoef_SWB_AVQ, const Short *ord_bands, Short *avqType);
static void globalGainAdj (Short *avqType, Float *fcoef_SWB_AVQ, Float *fcoef_SWB_AVQ_abs, Float *fenv_BWE);
static Short getBandLx_decodAVQ_flt(Short *smdct_coef_Lx, Float *fmdct_coef_AVQ, Short *bandTmp, Short nbBand, Short *bandLx, Short *bandZero);
static Float f_invEnv_BWE(Float sEnv, Short expx, Short *exp_num);
static Short f_Compute_Corr(const Float vec_base[], const Float vec_fill[]);
static Float f_getParamFillBand(Float *svec_base, Float *vec_fill, Short expx, Short *ind_corr_max );
static void getBaseSpectrum_flg1(Short *avqType, Float *fenv_BWE, Float *fEnv_BWE, Float *fvec_base, Float *fcoef_SWB_AVQ_abs, 
                                 Float *fcoef_SWB_AVQ, Float *fcoef_SWB);
static void fillZeroBands_flg1(Short *avqType, Short *iZero, Float *fcoef_SWB, Float *fcoef_SWB_AVQ_abs, Float *fenv_BWE, Float *fvec_base, 
                               long *indexL, Short *nbBitsL);
static void getVecToFill_flg1( Float fenv_BWE, Float *fcoef_SWB, Float *vecToFill);
static void getBaseSpectrum_flg0 (Short *avqType, Float *fmdct_coef_avq, Float *fvec_base);
static void fillZeroBands_flg0(Short Qval, Short *avqType, 
                               Float *fmdct_coef_nq, Float *fmdct_coef_avq, Float *fvec_base, 
                               long *indexL, Short *nbBitsL);
static void getSignInfo( Short *avqType, Float *smdct_coef, Short *mdct_sign, Short *nbBitsRestL1, Short *nbBitsRestL2,
                        long *indexL1, Short *nbBitsL1, long *indexL2, Short *nbBitsL2);
static Short getSignIndex(Float *x, Short signIn, Short *signOut);
static void allocateSignInfo(Short *nbBitsSign, Short nbBandL, Short *nbBits,Short *signIn, Short *nbSignIn,
                             long *index, Short *nbBitLx);

/* Constructor for AVQ encoder */
void* avq_encode_const (void)
{
  AVQ_state_enc *enc_st = NULL;

  enc_st = (AVQ_state_enc *) malloc (sizeof(AVQ_state_enc));
  if (enc_st == NULL) return NULL;

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
}

Short avq_encode_reset (void *work)
{
  AVQ_state_enc *enc_st = (AVQ_state_enc *) work;

  if (enc_st != NULL)
  {
    /* initialize each member */
    zeroS(sizeof(AVQ_state_enc)/2, (Short *)work);
  }
  return ENCODER_OK;
}

static void encoder_SWBL1L2_AVQ( 
                                const float *fmdct_coef,    /* i:    MDCT coefficients to encode     */
                                unsigned short **pBst_L1,  /* i:  pointer to L1 bitstream buffer        */
                                unsigned short **pBst_L2,       /* i:    pointer to L2 bitstream buffer*/
                                const Short layers,          /* i:    number of swb layers encoded  */
                                Short savqType[],      /* o: Output vector signalising zero bands */
                                float *fcoef_SWB_AVQ,  /* o:    locally decoded MDCT coefs. */
                                Short *unbits_L1,
                                Short *unbits_L2/* i */
                                )
{
  Short ib, i;
  Short smdct_coef_norm_L1[(WIDTH_BAND+1)*N_SV_L1], smdct_coef_norm_L2[(WIDTH_BAND+1)*N_SV_L2];
  Short *sbandL1, *sbandL2, *sbandZero, sbandTmp[N_SV];
  Short nbBandZero;
  Short *sbandLx; 
  
  Float *ptr0, *ptr1;
  Float fmdct_coef_L2[WIDTH_BAND*N_SV_L2];

  sbandL1 = savqType+3;
  sbandL2 = sbandL1 + N_SV_L1;
  sbandZero = sbandL2 + N_SV_L2+2;
  /* SWBL1 AVQ encoder */
  AVQ_cod( (Float *)fmdct_coef, smdct_coef_norm_L1, N_BITS_AVQ_L1, N_SV_L1 );

  *unbits_L1 = AVQ_encmux_bstr( smdct_coef_norm_L1, pBst_L1, N_BITS_AVQ_L1, N_SV_L1 );

  /* get bands coded with L1 and zero bands and local decoding of SWBL1 */
  for(i=0; i<N_SV; i++)
  {
    sbandTmp[i] = i; 
  }
  sbandLx = sbandL2;
  if (layers == 1)
  {
    sbandLx = sbandZero;
  }
  savqType[0] = getBandLx_decodAVQ_flt(smdct_coef_norm_L1, fcoef_SWB_AVQ, sbandTmp, N_SV_L1, sbandL1, sbandLx); 

  nbBandZero = N_SV_L1 - savqType[0];
  if (layers == 2)
  {
    /* form bands to be coded with L2 */
    movSS(N_SV_L2 - nbBandZero, &sbandTmp[N_SV_L1], &sbandL2[nbBandZero]);
    ptr1= fmdct_coef_L2;
    for(ib= 0; ib<N_SV_L2; ib++)
    {
      i = sbandL2[ib]; 
      ptr0 = (Float*)fmdct_coef + (i<<3);
      movF(WIDTH_BAND, ptr0, ptr1);
      ptr1 += WIDTH_BAND;
    }

    /* SWBL2 AVQ encoder */
    AVQ_cod( (Float *)fmdct_coef_L2, smdct_coef_norm_L2, N_BITS_AVQ_L2, N_SV_L2 );
  
    *unbits_L2 = AVQ_encmux_bstr(smdct_coef_norm_L2, pBst_L2, N_BITS_AVQ_L2, N_SV_L2);

    /* get bands coded with L2 and zero bands and local decoding of SWBL2 */
    savqType[1]= getBandLx_decodAVQ_flt(smdct_coef_norm_L2, fcoef_SWB_AVQ, sbandL2, N_SV_L2, sbandL2, sbandZero);
    
    movSS(N_SV-N_SV_L2-savqType[0], &sbandTmp[N_SV-N_SV_L1-1+savqType[0]], &sbandZero[N_SV_L2 - savqType[1]]);
    nbBandZero = N_SV - savqType[0] - savqType[1];
  }
  else {
    nbBandZero = N_SV - savqType[0];
    movSS(N_SV-N_SV_L1, &sbandTmp[N_SV_L1], &sbandZero[N_SV_L1 - savqType[0]]);
    *unbits_L2 = 0;
  }
  for(ib=0; ib<nbBandZero; ib++) 
  {
    ptr1 = fcoef_SWB_AVQ + (sbandZero[ib] << 3);    
    zeroF(WIDTH_BAND, ptr1);
  }
  savqType[2] = nbBandZero;
   
  return;
}

static Float f_getParamFillBand (Float *fvec_base, Float *f_vec_fill, Short expx, 
                                Short *indCorr)
{
  int i;
  Short ind_corr_max;
  Float f_Gain16; 
  Float f_en, f_tmp;

  ind_corr_max  = f_Compute_Corr( fvec_base, f_vec_fill);
  f_Gain16 = -1.0f;
  f_en = 0.0f;
  f_tmp = 0.0f;
  /* reconstruct the zero subband */
  if( ind_corr_max < CORR_RANGE_L1 )
  {
	  for( i=0 ; i<WIDTH_BAND ; i++ ){
		  f_en += ( fvec_base[i] * fvec_base[i] );
	  }
	  f_tmp = Sqrt(f_en);
	  f_Gain16 = f_tmp;
  }
  *indCorr = ind_corr_max;

  return (f_Gain16);
}

static Short f_Compute_Corr(
                           const Float vec_base[],
                           const Float vec_fill[]
)
{
  Short i, ind_max;
  Float corr, corr_max;
  const Float *ptr;

  /* compute correlations */
  ptr = vec_base;
  corr_max = 0.0f;
  ind_max = CORR_RANGE_L1;
  for( i=0; i<CORR_RANGE_L1; i++ )
  {
	  corr = mac_Array_f( WIDTH_BAND ,  (Float*)ptr , (Float*)vec_fill );
	  ptr++;
	  if( corr > corr_max )
    {
      ind_max = i; 
    }
	  if( corr_max >= corr)
		  corr_max = corr_max;
	  else
		  corr_max = corr;
  }
  return( ind_max );
}

static void compute_sratio (Float *fEnv_BWE, Float *fratio)
{
	int i;

  for(i=0; i<N_SV; i++)
  {
    if(fratio[i] == 0){ fratio[i] = 1.0f;}
	fratio[i] = ( fEnv_BWE[i] / fratio[i] ) - 1.0f;
  }
  return;
}

/* calculate ksm and absolute value of mdct coefficients */
static Short compute_ksm(Float *fYb, Float *fy_s_abs, Short *sord_b, 
                          AVQ_state_enc *w_AVQ_state_enc) 
{
  Short i,n, flg_bit;
  Float j,k;
  Float *f_ptr;

  /* calculate abs. value of Yb */
  abs_array_f(fYb, fy_s_abs, SWB_F_WIDTH);
  k = 0;
  f_ptr = fy_s_abs;
  for(i=0; i<N_SV; i++)
  {    
    if( sord_b[i] < TH_ORD_B )
    { 
      for( n=0; n<WIDTH_BAND; n++ )
      {
        if( f_ptr[n] < 0.5f )
        {
          k += 1.0f;
        }
      }
    }
    f_ptr += WIDTH_BAND;
  }
  flg_bit = 0;
  /* ksm *= (1.f-SMOOTH_K);         */
  j = w_AVQ_state_enc->fksm * 0.70001220703125f;
  /* ksm += (SMOOTH_K * (float) k); */
  w_AVQ_state_enc->fksm = j + (k * 0.29998779296875f);
  /* select encoding mode */
  /* if ( mnl == LOW_LEVEL_NUM_MIN ) */

  if( w_AVQ_state_enc->fksm <= 15.0f )
  {
    /* flag  = 1 */
    flg_bit += 1;
  }
  else
  {
    if( w_AVQ_state_enc->fksm < 20.0f )
    {
      /* flag  = 1 */
      flg_bit = w_AVQ_state_enc->s_mnl;
    }
  }
  w_AVQ_state_enc->s_mnl = flg_bit ;
  return (flg_bit);
}

static void compute_mdct_err (Float *fy_s_abs, Float *fmdct_err, Float *fcoef_SWB,
                              Short *mdct_sign, Float *fenv_BWE_err, Float *fenv_BWE)
{
  Float *p_fy, *p_fe, *ptr0;
  Short signIndex;
  Short i, j;
  Float ftmp, tmp;

  p_fy = fy_s_abs;
  p_fe = fmdct_err;
  ptr0 = fcoef_SWB;
  for(i=0; i<N_SV; i++)
  {
    signIndex = 0;
    ftmp = 0.5f * fenv_BWE[i];
    for( j=0; j<WIDTH_BAND; j++ ) 
    {
      /* get sign info */
      signIndex = signIndex << 1;
      if (*ptr0 >= 0.0f)
      {
        signIndex += 1;
      }
      /* calculate MDCT error */
      tmp = *(p_fy++) - ftmp; 
      /* delete negative component */
      tmp = f_max (0.0f , tmp);
      if (*ptr0 < 0.0f)
      {
		tmp = -tmp;
      }
      *p_fe++ = tmp;
      ptr0++;
    }
    mdct_sign[i] = signIndex;
    /* calculate Fenv_BWE_err */
    /* ---------------------- */
    fenv_BWE_err[i] = 0.600006103515625f * fenv_BWE[i];
  }
  return;
}

static Short detectPbZeroBand_flg1(Short nbBandZero, Short *bandZero, 
                                    Short unbits_L1, Float *fratio, Short *nbBits)
{
  Short detprob_flg;
  Short ib, i, nb;

  Float fmax_ratio;

  /* determine on how many bits detprob_flg can be written; nb_bits = 0, 1, or 2 */
  nb = 0;
  if(unbits_L1 > 0) 
  {
    nb += 1;
    if( unbits_L1 > 1 )
    {
      nb += 1;
      if( unbits_L1 == N_BITS_FILL_L1+1 ) 
      {
        nb -= 1;
      }
    }
  }
  detprob_flg = 0;
  if( nb > 0 ) 
  {
    fmax_ratio = -8.0f;
    for( ib=0; ib<nbBandZero; ib++ )
    {
      i = bandZero[ib];
	  if( fratio[i] >= fmax_ratio)
		  fmax_ratio = fratio[i];
	  else
		  fmax_ratio = fmax_ratio;
    }
    if( fmax_ratio > 2.0f )
    {
      detprob_flg += 1;        
      if( fmax_ratio > 4.0f )
      {
        detprob_flg += 1;        
        if( fmax_ratio >= 8.0f ){
          detprob_flg += 1;        
        }
      }
    }
    if( nb == 1 ) 
    {
	  if( detprob_flg <= 1)
		  detprob_flg = detprob_flg;
	  else
		  detprob_flg = 1;
    }
  }
  *nbBits = nb;

  return(detprob_flg);
}

static void getSignInfo( Short *avqType, Float *fmdct_coef, Short *mdct_sign,
                        Short *nbBitsRestL1, Short *nbBitsRestL2, long *indexL1, Short *nbBitsL1, 
                        long *indexL2, Short *nbBitsL2)
{
  Short ib, i, nbBitSign, *bandL1, nbZero[N_SV], signIndex[N_SV];
  Float *ptr0;

  bandL1 = avqType + 3;
  nbBitSign   = 0;
  for(ib=0; ib<avqType[0]; ib++)
  {
    i = bandL1[ib];
    ptr0 = fmdct_coef + (i * 8);
    nbZero[ib]= getSignIndex(ptr0, mdct_sign[i], &signIndex[ib]);
    nbBitSign = nbBitSign + nbZero[ib];
  }
  if(*nbBitsRestL1 > 0) 
  {
    allocateSignInfo(&nbBitSign, avqType[0], nbBitsRestL1, signIndex, nbZero, indexL1, nbBitsL1);
  }
  if(*nbBitsRestL2 > 0) 
  {
    allocateSignInfo(&nbBitSign, avqType[0], nbBitsRestL2, signIndex, nbZero, indexL2, nbBitsL2);
  }
  return;
}

/* allocate sign information */
static void allocateSignInfo(Short *nbBitSign, Short nbBand, Short *nbBits, Short *signIn, Short *nbSignIn, 
                             long *index, Short *nbBitLx)
{
  Short ib; 
  Short n, signIndex, nbBandSign;
  Short nbBitSignbuf;

  nbBandSign = nbBand;
  nbBitSignbuf = *nbBitSign;
  if( nbBitSignbuf > *nbBits )
  {
    nbBandSign -= 1; 
    n = nbBandSign;
    for(ib=n; ib>=0; ib--)
    {
      nbBitSignbuf -= nbSignIn[ib];
      if( nbBitSignbuf > *nbBits ) 
      {
        nbBandSign -= 1;
      }
    }
  }
  for(ib =0; ib<nbBandSign; ib++)
  {
    s_getIndexBitstream(nbSignIn[ib], signIn[ib], nbBitLx, index);
    *nbBits -= nbSignIn[ib];
    *nbBitSign -= nbSignIn[ib];
    nbSignIn[ib] = 0;
    signIn[ib] = 0;
  }
  if( nbBand > nbBandSign ) 
  {
    n = nbSignIn[nbBandSign] - *nbBits;
    if( n >=0 ) 
    {
      signIndex = signIn[nbBandSign] >> n;
      s_getIndexBitstream(*nbBits, signIndex, nbBitLx, index);
      *nbBitSign -= *nbBits;
      nbSignIn[nbBandSign] = n;
      signIn[nbBandSign] = signIn[nbBandSign] & ((1 << n)-1);
      *nbBits = 0;
    }
  }
  return;
}

/*--------------------------------------------------------------------------*
*  Function  swbl1_encode_AVQ()		                                         *
*  ~~~~~~~~~~~~~~~~~~~~~~~~~~~~                                            *
*  Main function for encoding Extension layers SWBL1 and SWBL2             *
*--------------------------------------------------------------------------*/
int swbl1_encode_AVQ (
                         void* p_AVQ_state_enc,			/* (i/o): Work space							*/
                         const Float  fcoef_SWB[],		/* i:	Input SWB MDCT coefficients			    */
                         const Float  fEnv_BWE[],		/* i:	Input frequency envelope from SWBL0     */
                         Float fratio[],				/* i:	Unquantized input frequency envelope    */
                         const Short  index_g_5bit,		/* i:	5 bit index of frame gain from SWBL0    */
                         const Short  cod_Mode,			/* i:	mode information from SWBL0             */
                         unsigned short *pBst_L1,       /* o:	Output bitstream for SWBL1              */
                         unsigned short *pBst_L2,       /* o:	Output bitstream for SWBL2              */
                         const Short  layers			/* i:	number of swb layers transmitted		*/
                         )
{
  int i;
  Short j, ib, index_gain;
  Short flg_bit;
  Short flgMode;
  Float fenv_BWE[N_SV];
  Float f_Yb[SWB_F_WIDTH];
  Float fmdct_coef[SWB_F_WIDTH];
  Float fFenv_BWE[N_SV];
  Float *ptr_fykr, *ptr_fenv;
  Short s_ord_bands[N_SV];
  Float f_norm_ratio[N_SV]; /* buffer for Sort() */
  Float fmdct_err[SWB_F_WIDTH];
  Float fenv_BWE_err[N_SV];
  Float fcoef_SWB_AVQ[SWB_F_WIDTH], fcoef_SWB_AVQ_abs[SWB_F_WIDTH];
  Float fy_s_abs[SWB_F_WIDTH];
  Float fvec_base[3*WIDTH_BAND]; 
  Float *f_ptr;
  Float fGopt;
  Short inc_cnt_detzer;
  Short flg_L1, flg_L2, flg_fill;
  Short tmp;
  Short unbits_L1, unbits_L2;
  Short mdct_sign[N_SV];
  Short avqType[N_SV_L1+N_SV_L2+2+N_SV+3];
  Short *bandL1, *bandL2, *bandZero;
  Short *ptr;
  long  indexL1, indexL2;
  Short nbBitsL1, nbBitsL2;
  Short detprob_flg;
  Short nbBit_detprob_flg;
  Short nbBitsRestL1, nbBitsRestL2;
  unsigned short *pBst_g;
  Float fGglob;

  AVQ_state_enc *w_AVQ_state_enc = (AVQ_state_enc *)p_AVQ_state_enc;

  zeroS(N_SV_L1+N_SV_L2+2+N_SV+3, avqType);
  bandL1 = avqType+3;
  bandL2 = bandL1 + N_SV_L1;
  bandZero = bandL2 + N_SV_L2+2;

  /* calculate subband energy */
  f_loadSubbandEnergy (cod_Mode, (Float *)fEnv_BWE, fFenv_BWE , index_g_5bit);

  /* compute  sratio */
  compute_sratio (fFenv_BWE, fratio);

  /* Normalize by global gain */
  fGglob = 1.0f / Pow( 2.0f , (Float)index_g_5bit );
  for( i=0 ; i<SWB_F_WIDTH ; i++ ){
	  f_Yb[i] = fcoef_SWB[i] * fGglob;
  }

  for( i=0 ; i<N_SV ; i++ ){
	  fenv_BWE[i] = fFenv_BWE[i];
  }

  /* order spectral envelope subbands by decreasing perceptual importance */
  /* order subbands by decreasing perceptual importance  */
  f_Sort( fFenv_BWE, N_SV, s_ord_bands, f_norm_ratio );
  flg_bit = 0;
  ptr_fykr = f_Yb;
  ptr_fenv = fenv_BWE;
  flgMode= cod_Mode | w_AVQ_state_enc->s_pre_cod_Mode;

  if(flgMode == 0)
  {
    /* write flag_bit to bitstream */
	*pBst_L1 = ITU_G192_BIT_0;
	flg_bit = compute_ksm(f_Yb, fy_s_abs, s_ord_bands, w_AVQ_state_enc);

    if(flg_bit != 0) 
    {  
      /* write flag_bit to bitstream */
	  *pBst_L1 = ITU_G192_BIT_1;

      /* compute MDCT error and sign information */
      compute_mdct_err (fy_s_abs, fmdct_err, (Float *)fcoef_SWB, mdct_sign, 
        fenv_BWE_err, fenv_BWE);

      ptr_fykr = fmdct_err;
      ptr_fenv = fenv_BWE_err;
    }
	pBst_L1++;
    pBst_g = pBst_L1 + N_BITS_AVQ_L1;
  }
  else
  {
    pBst_g = pBst_L1 + N_BITS_AVQ_L1_PLS;
  }

  /* normalize per band, amplify for AVQ normalization & forward reorder of subbands */
  bandNormalize_Order_flt( ptr_fykr, fmdct_coef, ptr_fenv, s_ord_bands);

  /* ***** apply algebraic vector quantization (AVQ) to MDCT coefficients * */
  encoder_SWBL1L2_AVQ( fmdct_coef, &pBst_L1, &pBst_L2, layers, avqType, fcoef_SWB_AVQ, &unbits_L1, &unbits_L2);

  pBst_L1 -= unbits_L1;
  pBst_L2 -= unbits_L2;
  indexL1 = 0L;
  indexL2 = 0L;
  nbBitsL1 = 0;
  nbBitsL2 = 0;

  if( flgMode != 0 )
  {
    unbits_L1 = unbits_L1 + 1;
  }

  if( flg_bit == 0 )
  {
    /* case flg_bit = 0 */
    /* ***** detect frames with problematic zero subbands ***** */
    inc_cnt_detzer = -3;
    if( layers == 2 )
    {
	  inc_cnt_detzer= detectPbZeroBand_flg0(ptr_fykr, fratio, s_ord_bands, bandZero, avqType[2], w_AVQ_state_enc->s_cnt_detzer);
    }
    w_AVQ_state_enc->s_cnt_detzer = w_AVQ_state_enc->s_cnt_detzer + inc_cnt_detzer;
	if( w_AVQ_state_enc->s_cnt_detzer > 0 )
		tmp = w_AVQ_state_enc->s_cnt_detzer;
	else
		tmp = 0;
	if( w_AVQ_state_enc->s_cnt_detzer < DETZER_MAX )
		w_AVQ_state_enc->s_cnt_detzer = tmp;
	else
		w_AVQ_state_enc->s_cnt_detzer = DETZER_MAX;
    if( unbits_L1 > 0 )
    {
		if( w_AVQ_state_enc->s_cnt_detzer < 1 )
			w_AVQ_state_enc->s_detzer_flg = w_AVQ_state_enc->s_cnt_detzer;
		else
			w_AVQ_state_enc->s_detzer_flg = 1;
      s_getIndexBitstream(1, w_AVQ_state_enc->s_detzer_flg, &nbBitsL1, &indexL1 );
    }

    /* ***** try to find a filling of zero subbands ***** */
    if( layers == 2 )
    {
      if( w_AVQ_state_enc->s_detzer_flg==0)
      {
        flg_L1 = unbits_L1 - (nbBitsL1 + N_BITS_FILL_L1);
        flg_L2 = unbits_L2 - N_BITS_FILL_L2;
		if( flg_L1 >= flg_L2)
			flg_fill = flg_L1;
		else
			flg_fill = flg_L2;
        if( flg_fill >= 0 ) 
        {
          if( (avqType[0] + avqType[1] - N_BASE_BANDS) >= 0 )
          {
            /* form base spectrum */
            getBaseSpectrum_flg0(avqType, fcoef_SWB_AVQ, fvec_base);
            /* try to fill zero band */
            if( flg_L1 >= 0 )
            {
			  fillZeroBands_flg0(0, avqType, fmdct_coef, fcoef_SWB_AVQ,
                fvec_base, &indexL1, &nbBitsL1);
            }
            if( flg_L2 >= 0 )
            {
              if( avqType[2] > 0 ) 
              {
				fillZeroBands_flg0(QCOEF, avqType, fmdct_coef, fcoef_SWB_AVQ, 
                  fvec_base, &indexL2, &nbBitsL2);
              }
            }
          }
        }
      }
    }

	/* backward reordering */
    ptr = avqType + 3+ N_SV_L1;
    s_sortIncrease(avqType[1], avqType[1], ptr, ptr);
    bwdReorder (ptr_fenv, fcoef_SWB_AVQ, fmdct_coef, s_ord_bands, avqType);

    /* compute global gain adjustment */
    fGopt = ggain_adj ( bandL1, avqType[0], f_Yb, fmdct_coef, index_g_5bit);

	/* Embedded coding of the adjusted gain in log2 domain */
    index_gain = cod_emb_fgain (index_g_5bit, &fGopt);

	if( layers == 2 )
    {
      /** calculate locally decoded MDCT coefs.---------------*/               
      /* for zero subbands, keep MDCT coeficients from the BWE SWBL0 */	
      fGglob = Pow(2.0f, (Float)index_g_5bit);
      fGopt *= fGglob;
      for( ib=0 ; ib<avqType[0] ; ib++ )
      {
		f_ptr = fmdct_coef + (bandL1[ib] * 8);
        /* apply adjusted global gain to AVQ decoded MDCT coefs */
        for( j=0 ; j<WIDTH_BAND ; j++)
        {
		  f_ptr[j] = f_ptr[j] * fGopt;
        }
      }
      for( ib=0 ; ib<avqType[1] ; ib++ )
      {
		f_ptr = fmdct_coef + (bandL2[ib] * 8);
		for( i=0 ; i<WIDTH_BAND ; i++ ){
			f_ptr[i] = f_ptr[i] * fGglob;
		}
      }

      /*-calculate gradient and modify locally decoded MDCT coefs. */   
	  encoder_coef_SWB_AVQ_adj(fcoef_SWB, bandL1, avqType[0], fmdct_coef, 
        &indexL1, &nbBitsL1, (unbits_L1 - nbBitsL1) );
      encoder_coef_SWB_AVQ_adj(fcoef_SWB, bandL2, avqType[1],fmdct_coef, 
        &indexL2, &nbBitsL2, (unbits_L2 - nbBitsL2));

      nbBitsRestL1 = unbits_L1 - nbBitsL1;
      nbBitsRestL2 = unbits_L2 - nbBitsL2;
    }
  }
  else 
  {
    /* case flg_bit = 1 */
    /* backward reordering */
    bwdReorder (ptr_fenv, fcoef_SWB_AVQ, fmdct_coef, s_ord_bands, avqType);

    /* compute global gain adjustment */
    /* ------------------------------ */
    globalGainAdj(avqType, fmdct_coef, fcoef_SWB_AVQ_abs, fenv_BWE);

    fGopt = ggain_adj (bandL1, avqType[0], fy_s_abs, fcoef_SWB_AVQ_abs, index_g_5bit);

    if( layers == 2 )
    {
      detprob_flg = detectPbZeroBand_flg1(avqType[2], bandZero, unbits_L1, fratio, &nbBit_detprob_flg );
      s_getIndexBitstream( nbBit_detprob_flg, detprob_flg, &nbBitsL1, &indexL1 );
    }
    w_AVQ_state_enc->s_cnt_detzer = w_AVQ_state_enc->s_cnt_detzer - 3;
	if( w_AVQ_state_enc->s_cnt_detzer > 0 )
		tmp = w_AVQ_state_enc->s_cnt_detzer;
	else
		tmp = 0;
	if( w_AVQ_state_enc->s_cnt_detzer < DETZER_MAX )
		w_AVQ_state_enc->s_cnt_detzer = tmp;
	else
		w_AVQ_state_enc->s_cnt_detzer = DETZER_MAX;

    if( layers == 2 )
    {
	  Short tmp;
      flg_L1 = unbits_L1 - (nbBitsL1 + N_BITS_FILL_L1);
      flg_L2 = unbits_L2 - N_BITS_FILL_L2;
	  if( flg_L1 >= flg_L2)
			flg_fill = flg_L1;
		else
			flg_fill = flg_L2;
      if( flg_fill >= 0 ) 
      {
        if( ((avqType[0] + avqType[1]) - N_BASE_BANDS) >= 0 )
        {
		  if( 2 <= avqType[2] )
			  tmp = 2;
		  else
			  tmp = avqType[2];
          getBaseSpectrum_flg1 (avqType, fenv_BWE, (Float *)fEnv_BWE, fvec_base, fcoef_SWB_AVQ_abs, fmdct_coef, (Float *)fcoef_SWB);

          s_sortIncrease(avqType[2], tmp, bandZero, bandZero); 
          if( flg_L1 >= 0 )
          {
            fillZeroBands_flg1(avqType, bandZero, (Float *)fcoef_SWB, fcoef_SWB_AVQ_abs, fenv_BWE, fvec_base, &indexL1 , &nbBitsL1);
          }
          if( flg_L2 >= 0 )
          {
            fillZeroBands_flg1(avqType, bandZero, (Float *)fcoef_SWB, fcoef_SWB_AVQ_abs, fenv_BWE, fvec_base, &indexL2 , &nbBitsL2);
          }
        }
		else
        {
          if( flg_L1 >= 0 )
          {
            s_getIndexBitstream(N_BITS_FILL_L1, 0, &nbBitsL1, &indexL1);
          }
        }
      }
    }

    /*-- send sign information----------------------------*/  
    /* set pointer adress */ 
    if( layers == 1 )
    {
      nbBitsL1 = unbits_L1;
	  if( N_BITS_FILL_L1+2 <= nbBitsL1 )
		  nbBitsL1 = N_BITS_FILL_L1+2;
	  else
		  nbBitsL1 = nbBitsL1;
    }
    nbBitsRestL1 = unbits_L1 - nbBitsL1;
    nbBitsRestL2 = unbits_L2 - nbBitsL2;

	if( nbBitsRestL1 >= nbBitsRestL2 )
		tmp = nbBitsRestL1;
	else
		tmp= nbBitsRestL2;
    if( tmp > 0) 
    { 
      /* allocate sign information */
      s_sortIncrease(avqType[0], avqType[0], bandL1, bandL1); 
      getSignInfo( avqType, fmdct_coef, mdct_sign, &nbBitsRestL1, &nbBitsRestL2,
        &indexL1, &nbBitsL1, &indexL2, &nbBitsL2);

    }

    /* Embedded coding of the adjusted gain in log2 domain */
    index_gain = cod_emb_fgain (index_g_5bit, &fGopt);
  }
  s_PushBitLong( indexL1, &pBst_L1, nbBitsL1);
  s_PushBit( index_gain, &pBst_g, N_BITS_GAIN_SWBL1);
  s_PushBitLong( indexL2, &pBst_L2, nbBitsL2);
  pBst_L2 += nbBitsRestL2;
  w_AVQ_state_enc->s_pre_cod_Mode = cod_Mode;

  return 0;
}

/*-----------------------------------------------------------------*
*   Function  ggain_adj                                           *
*                                                                 *
*   Compute global gain adjustment                                *
*-----------------------------------------------------------------*/
static Float ggain_adj(
                        Short *bandL1, Short nbBandL1, Float *f_sx,
                        Float *f_sqx, Short index_g_5bit)
{
  Short ib, i, j;
  Float *f_px,*f_qx,f_Gaf;
  Float f_lnum, f_lden,f_lnumtmp, f_ldentmp;
  Float f_den, f_num;

  f_lnum = 0.0f;
  f_lden = 0.000030517578125f;

  for(ib=0; ib<nbBandL1; ib++)
  {
    i= bandL1[ib];
    i *= 8;
	f_px = f_sx+ i;
    f_qx = f_sqx + i;
	f_lnumtmp = 0.0f;
    f_ldentmp = 0.0f;
    for(j=0; j<WIDTH_BAND; j++)
    {
      f_lnumtmp += (f_px[j] * f_qx[j]);
      f_ldentmp += (f_qx[j] * f_qx[j]); 
    }
    f_lnum += f_lnumtmp;
    f_lden += f_ldentmp;
  }

  f_den = 1.0f / f_lden;
  f_num = f_lnum;

  f_Gaf = f_num * f_den;

  return(f_Gaf);
}

static Short cod_emb_fgain (Short index_g_5bit,
                             Float *fgopt)
{
  Short i, min_index_frac;
  Float fgopt_q;
  Float fdtmp, fmin_dist;

  /* Additional 3bit search of frame gain */
  if(index_g_5bit == 0)
  {
    min_index_frac = minDiff0Array16(5, *fgopt, (Float *)f_sg0, &fmin_dist); 
    fgopt_q = f_sg0[min_index_frac];
    i = minDiff0Array16(3, *fgopt, (Float *)&fgain_frac[5], &fdtmp); 
    if( fdtmp <= fmin_dist )
    {
      min_index_frac = i + 5;
      fgopt_q = fgain_frac[min_index_frac];
    }
  }
  else
  {
    min_index_frac = minDiff0Array16(8, *fgopt, (Float *)fgain_frac, &fmin_dist); 
    fgopt_q = fgain_frac[min_index_frac];
  }
  *fgopt = fgopt_q ;

  return(min_index_frac);
}

static Short minDiff0Array16( Short n, Float x, Float *y, Float *Lmin_dist)
{
  Short i, min_index;
  Float tmp;
  Float fdtmp, fmin_dist;
 
  /* Additional 3bit search of frame gain */
  min_index = 0;
  fmin_dist = 2147483647.0f;
  for(i=0; i<n; i++) 
  {
    tmp = x - y[i];
    fdtmp = tmp * tmp;
    if( fdtmp <= fmin_dist )
    {
      min_index = i;
    }
	if( fmin_dist <= fdtmp)
		fmin_dist = fmin_dist;
	else
		fmin_dist = fdtmp;
  }
  *Lmin_dist = fmin_dist;

  return(min_index);
}

/*-----------------------------------------------------------------*
*   Funtion  encoder_coef_SWB_AVQ_adj                             *
*            ~~~~~~~~~~~~~~~~~~~~~~~~                             *
*   calculate gradient to modify locally decoded MDCT coefs.       *
*-----------------------------------------------------------------*/
static void encoder_coef_SWB_AVQ_adj( 
                                     const Float *fcoef_SWB,     /* i:  MDCT coefficients to encode             */
                                     Short *bandL, /* i:  input vector signalising AVQ type  (0, L1, L2) */
                                     Short  nbBand, /* i:  nb bands of AVQ type  (0, L1, L2) */
                                     Float *fcoef_SWB_AVQ, /* i/o:  locally decoded MDCT coefs.            */ 
                                     long  *indexL,
                                     Short *nbBitsL,
                                     Short  unbits
                                     )                        
{
  Short ig, i8, ib, n, nbGrad2, nbGrad1, max_idx;
  Short idx, index, bit_alloc;
  Short *ptrBand;

  Float *ptr0, *ptr1;

  if(unbits > 0) 
  {
    /* calculate the number of bands with  2 bits gradient */
    n = unbits - nbBand;
	if( nbBand <= n)
      nbGrad2 = nbBand;
	else
      nbGrad2 = n;

	if( nbGrad2 >= 0)
      nbGrad2 = nbGrad2;
    else
      nbGrad2 = 0;
	
    if( nbBand <= unbits)
      nbGrad1 = nbBand;
    else
      nbGrad1 = unbits;
	nbGrad1 -= nbGrad2;

    /* calculate gradient */
    /* ------------------ */
    bit_alloc = nbGrad2 * 2;
    index = 0;
    n = nbGrad2;
    max_idx = 3;
    ptrBand = bandL;
    for(ig = 2; ig>0; ig--)
    {
      for(ib=0; ib<n; ib++)
      {
        /* calculate gradient of each vector */
        i8 = *ptrBand++ * 8;
        ptr0 = (Float*)fcoef_SWB+ i8;
        ptr1 = fcoef_SWB_AVQ + i8;
        idx = compute_errGradNormL1(ptr0, ptr1, (Float*)fgrad, max_idx);
        index = index * (Short) Pow(2.0f , (Float)ig);
        index += idx;
      }
      n = nbGrad1;
      max_idx -= 2;
    }
    bit_alloc += nbGrad1;
    s_getIndexBitstream(bit_alloc, index, nbBitsL, indexL);
  }
  return;
}

static Short compute_errGradNormL1(Float *x, Float *xq, Float *fgrad, Short max_idx)
{
  Short  k, j, idx;
  Float min_err, err;
  Float *ptr0, *ptr1;
  Float *ptr2;
  Float tmp;

  ptr0 = x;
  ptr1 = xq;
  /* calculate error between Yfb and coef_SWB_AVQ */
  min_err = 0;
  for(j=0; j<WIDTH_BAND; j++)
  {
    tmp = (*ptr0++) - (*ptr1++);
    min_err = min_err + abs_f(tmp);
  }
  /* compare errors */
  idx = 0;
  ptr2 = fgrad;
  for(k=0; k<max_idx; k++)
  {
    ptr0 -= WIDTH_BAND;
    ptr1 -= WIDTH_BAND;
    err = 0;
    for(j=0; j<WIDTH_BAND; j++)
    {
      err += abs_f( (*ptr0++) - ((*ptr1++) * (*ptr2++)) );
    }
    if( min_err > err ) 
    { 
      idx += 1; 
    } 
	if( err <= min_err)
		min_err = err;
	else
		min_err = min_err;
  }  
  return(idx);
}

static Short getSignIndex(Float *x, Short signIn, Short *signOut)
{
  Short j, nbSign, signIndex;
  Short mask;
  Float *ptr;

  mask = 0x1;
  signIndex = 0;
  ptr = x+ WIDTH_BAND-1;
  nbSign = 0;
  for(j=0; j<WIDTH_BAND; j++)
  {
    if(*ptr == 0)
    {
      signIndex = signIndex + ((mask & signIn) >> nbSign);
      nbSign -= 1;
    }
    nbSign += 1;
	mask *= 2;
    ptr--;
  }
  nbSign = WIDTH_BAND - nbSign;
  *signOut = signIndex;

  return(nbSign);
}

/* normalize per band, amplify for AVQ normalization & forward reorder of subbands */
static void bandNormalize_Order( const Float *fykr, Float *fmdct_coef, const Float *fenv_BWE, 
                                const Short *ord_bands)
{
  Short i, k, j;
  Float iGain16;

  Float *f_ptr0;
  const Float *f_ptr;

  f_ptr0 =  fmdct_coef; 
  for( i=0; i<N_SV; i++ )
  {
    /* initialization */
    k = ord_bands[i];
    f_ptr  = fykr + (k*8);
    if( fenv_BWE[k] > 0.0f )
    {
      /* Invert Gain */
      iGain16 = INV_CNST_WEAK_FX2_F / fenv_BWE[k];
      for( j=0; j<WIDTH_BAND; j++ )
      {
		*f_ptr0++ = *f_ptr++ * iGain16 / 4096.0f;
      }
    }
	else
    {
      zeroF( WIDTH_BAND, f_ptr0 );
      f_ptr0 += WIDTH_BAND;
    }
  }
  return;
}

/* normalize per band, amplify for AVQ normalization & forward reorder of subbands */
static void bandNormalize_Order_flt( const Float *fykr, Float *fmdct_coef, const Float *fenv_BWE, 
                                const Short *ord_bands)
{
  Short i, k, j;
  Float iGain16;

  Float *f_ptr0;
  const Float *f_ptr;

  f_ptr0 =  fmdct_coef; 
  for( i=0; i<N_SV; i++ )
  {
    /* initialization */
    k = ord_bands[i];
    f_ptr  = fykr + (k*8);
    if( fenv_BWE[k] > 0.0f )
    {
      /* Invert Gain */
      iGain16 = INV_CNST_WEAK_FX2_F / fenv_BWE[k];
      for( j=0; j<WIDTH_BAND; j++ )
      {
		*f_ptr0++ = *f_ptr++ * iGain16;
      }
    }
	else
    {
      zeroF( WIDTH_BAND, f_ptr0 );
      f_ptr0 += WIDTH_BAND;
    }
  }
  return;
}

/* ***** detect frames with problematic zero subbands ***** */
static Short detectPbZeroBand_flg0(const Float *fykr, const Float *fratio_fEnv, 
                                    const Short *ord_bands, Short *bandZero, Short nbBand0, Short cnt_detzer)
{
  Short detzer_flg1, detzer_flg2, inc_cnt_detzer;
  Short ib, i, j, k; 
  Float fmax_ratio,f_en,f_tmp,f_Lmax_band;
  Float f_mean_band, f_max_band; 
  const Float *f_ptr;

  detzer_flg1 = 0;
  fmax_ratio = 0.0f;
  for( ib=0; ib<nbBand0; ib++ )
  {
    i = bandZero[ib];
    k = ord_bands[i];
    fmax_ratio = f_max(fmax_ratio, fratio_fEnv[k]);
  }
  if( fmax_ratio > 4.0f )
  {
    detzer_flg1 = 1;
  }

  detzer_flg2 = 0;
  for( ib=0; ib<nbBand0; ib++ )
  {
    if( detzer_flg2 != 2 ) 
    {
      i = bandZero[ib];
      k = ord_bands[i];
      f_Lmax_band  = 0;
      f_tmp      = 0;
      f_ptr = fykr + ( k * 8);
      for( j = 0; j<WIDTH_BAND; j++ )
      {
		  f_en = f_ptr[j] * f_ptr[j];
		  f_tmp += f_en;
		  f_Lmax_band = f_max(f_en, f_Lmax_band);
      }
      /*mean_band /= WIDTH_BAND;*/
      f_mean_band = (Float)roundFto16(f_tmp);
      f_max_band = (Float)roundFto16(f_Lmax_band);
	  if( (f_max_band * 1.333251953125f) > f_mean_band ) 
      {               
        detzer_flg2 = 2;                       /* could be: leave the loop */
      }
      /* max_band > 4.0*mean_band )*/
	  else
      {
        if( (f_max_band, 4.0f) > f_mean_band )  
        {               
          detzer_flg2 = 1;
        }
      }
    }
  }
  if( nbBand0 < 5 ) 
  {
    detzer_flg2 = detzer_flg2 & 2; 
  }
  inc_cnt_detzer = DETZER_MAX;
  if( detzer_flg1 == 0 ) 
  {
    inc_cnt_detzer = detzer_flg2 - 2;
    if( (cnt_detzer > 0) )
    {
      if( inc_cnt_detzer == 0) {
        inc_cnt_detzer = inc_cnt_detzer + 3;
      }
    }
  }
  return(inc_cnt_detzer);
}

/* ***** try to find a filling of zero subbands ***** */
static void getBaseSpectrum_flg0 (Short *avqType, Float *fmdct_coef_avq, Float *fvec_base)
{
  Short bandLoc[N_SV_L1+N_SV_L2];
  Short ib, i;
  Float *ptr0, *ptr1;

  i = avqType[0];
  movSS(i, &avqType[3], bandLoc);
  movSS(avqType[1], &avqType[6], &bandLoc[i]);
  i += avqType[1];
  s_sortIncrease(i, 3, bandLoc, bandLoc);
  ptr1 = fvec_base;
  for(ib=0; ib<3; ib++) 
  {
    i = bandLoc[ib];
    ptr0 = fmdct_coef_avq + (i*8);
    movF(WIDTH_BAND, ptr0, ptr1);
    ptr1 += WIDTH_BAND;
  }

  return;
}

static void getBaseSpectrum_flg1(Short *avqType, Float *fenv_BWE, Float *fEnv_BWE , Float *fvec_base, 
                                  Float *fcoef_SWB_AVQ_abs, Float *fcoef_SWB_AVQ,
                                  Float *fcoef_SWB)
{
  Short ib, i8, i, j;
  Float *ptr0, *ptr1, *ptr2, *ptr;
  Short exp_num0, exp_num1, exp_num;
  Float Gain0, Gain1, ftmp;
  Short bandLoc[N_SV_L1+N_SV_L2];

  i = avqType[0];
  movSS(i, &avqType[3], bandLoc);
  movSS(avqType[1], &avqType[6], &bandLoc[i]);
  i += avqType[1];
  s_sortIncrease(i, 3, bandLoc, bandLoc);
  ptr = fvec_base;
  exp_num = 14;
  for(ib=0; ib<3; ib++) 
  {
    i = bandLoc[ib];
    i8 = i * 8;
    ptr0 = fcoef_SWB_AVQ_abs + i8;
    ptr2 = fcoef_SWB_AVQ + i8;
    ptr1 = fcoef_SWB + i8;
    /* Compute Gains 0 & 1 */
    Gain0 = f_invEnv_BWE(fEnv_BWE[i], exp_num, &exp_num0);
    Gain1 = f_invEnv_BWE(fenv_BWE[i], 2, &exp_num1);
    for( j=0; j<WIDTH_BAND; j++ )
    {
      if(ptr2[j] == 0.0f) 
      {
        ftmp = ptr0[j] * Gain0;
		if(ptr1[j] < 0.0f) ftmp = -ftmp;     
      }
	  else 
      {
        ftmp = ptr0[j] * Gain1;
		if(ptr2[j] < 0.0f) ftmp = -ftmp;;
      }
      ptr[j] = ftmp;
    }
    ptr += WIDTH_BAND;
  }
  return;
}

static void getVecToFill_flg1(Float fenv_BWE, Float *fcoef_SWB, Float *vecToFill)
{
  Short j, exp_num;
  Float Gain;

  /* inverse Fenv_BWE[i] */
  Gain = f_invEnv_BWE(fenv_BWE, 2, &exp_num);
  for( j=0; j<WIDTH_BAND; j++ )
  {
    /*vec_base[tmp16+j] = coef_SWB[i8+j]/Fenv_BWE[i];*/
    vecToFill[j] = fcoef_SWB[j] / fenv_BWE;
  }
  return;
}

static Float f_invEnv_BWE(Float sEnv, Short expx, Short *exp_num)
{
  Float tmpf, iGain16;

  tmpf = f_max(0.000244140625f, sEnv);  /* To ensure a non-zero value */
  iGain16 = 1.0f / tmpf;
  *exp_num = expx;  

  return (iGain16);
}

/* ***** try to find a filling of zero subbands ***** */
static void fillZeroBands_flg0(Short Qval, Short *avqType, 
                               Float *fmdct_coef_nq, Float *fmdct_coef_avq, Float *fvec_base, 
                               long *indexL, Short *nbBitsL)
{
  int i;
  Short i8, j, nbBand2, ind_corr_max;
  Short *ptrBand;
  Float *f_ptrBaseSpectrum, *f_vecToFill;
  Float Gain;

  /* ===== compute correlations for zero subband and reconstruction ===== */
  ptrBand = &avqType[3+N_SV_L1+N_SV_L2+2];
  i8 = ptrBand[0] * 8;
  f_vecToFill = fmdct_coef_nq + i8;

  Gain = f_getParamFillBand(fvec_base, f_vecToFill , Qval, &ind_corr_max);
  if(Gain >= 0.0f) 
  {
    f_ptrBaseSpectrum = fvec_base + ind_corr_max;
    f_vecToFill = fmdct_coef_avq + i8;
    if( Gain < 1.0f )
    {
      for( j=0; j<WIDTH_BAND; j++ )
      {
        f_vecToFill[j] = f_ptrBaseSpectrum[j] * Gain;
      }
    }
	else
    {
		for( i=0 ; i<WIDTH_BAND ; i++ ){
			f_vecToFill[i] = f_ptrBaseSpectrum[i];
		}
	}
    nbBand2 = avqType[1];
    ptrBand[nbBand2-(N_SV_L2+2)] = ptrBand[0];
    avqType[1] += 1;
    avqType[2] -= 1; 
    movSS(avqType[2], &ptrBand[1], ptrBand);
  }
  else
  {
    if( avqType[2] > 1 )
    {
      j = ptrBand[0];
      ptrBand[0]  = ptrBand[1];
      ptrBand[1] = j;
    }
  }

  s_getIndexBitstream( N_BITS_FILL_L1, ind_corr_max, nbBitsL, indexL);

  return;
}

static void fillZeroBands_flg1(Short *avqType, Short *iZero, Float *fcoef_SWB, Float *fcoef_SWB_AVQ_abs, Float *fenv_BWE, Float *fvec_base, 
                               long *indexL, Short *nbBitsL)
{
  Short i8, j;
  Short ind_corr_max; 
  Short *ptrBand;
  Short nbBand2;

  Float *f_ptr0,*f_ptr2;
  Float f_Gain16;

  ptrBand= &avqType[3+N_SV_L1+N_SV_L2+2];
  i8 = (*iZero) * 8;
  f_ptr0 = fcoef_SWB_AVQ_abs + i8;
  getVecToFill_flg1( fenv_BWE[*iZero], fcoef_SWB+i8, f_ptr0);
  /*  compute correlations for first zero subband and reconstruction ===*/
  f_Gain16 = f_getParamFillBand(fvec_base, f_ptr0, 0, &ind_corr_max);
  if(f_Gain16 >= 0.0f) 
  {
    f_ptr2 = fvec_base + ind_corr_max;
    if( f_Gain16 < 1.0f )
    {
      f_Gain16 = f_Gain16 * fenv_BWE[*iZero];
    }
	else 
    {
      f_Gain16 = fenv_BWE[*iZero];
    }
    for( j=0; j<WIDTH_BAND; j++ )
    {
      f_ptr0[j] = f_ptr2[j] * f_Gain16;
    }
    nbBand2 = avqType[1];
    ptrBand[nbBand2-(N_SV_L2+2)] = *iZero;
    avqType[1] += 1;

  }

  *iZero = ptrBand[1];
  s_getIndexBitstream( N_BITS_FILL_L1, ind_corr_max, nbBitsL, indexL);

  return;
}

static void bwdReorder (const Float *fenv_BWE, Float *fcoef_SWB_AVQ, Float *fmdct_coef, const Short *ord_bands, Short *avqType)
{
  Short ib, i, j, l;
  Short *ptr2;
  Short bandLoc[N_SV];
  Short inc, iavq, nbBand, *ptrBand;
  Float *ptr0, *ptr1;
  Float L_tmp;

  inc = 3;
  ptrBand = avqType + inc;
  for(iavq=0; iavq<3; iavq++) 
  {
    nbBand = avqType[iavq];
    ptr2 = bandLoc;
    for( ib=0; ib<nbBand; ib++ )
    {
      i = ptrBand[ib];
      j = ord_bands[i];
      *ptr2++ = j;
      ptr0 = fcoef_SWB_AVQ + (i * 8);
      ptr1 = fmdct_coef + (j * 8);
      for( l=0; l<WIDTH_BAND; l++)
      {
        L_tmp = ptr0[l] * fenv_BWE[j];
        ptr1[l]= L_tmp;
      }
    }
    movSS(nbBand, bandLoc, ptrBand);
    ptrBand += inc;
    inc += 3;
  }
  return;
}

static void globalGainAdj (Short *avqType, Float *fcoef_SWB_AVQ, 
                           Float *fcoef_SWB_AVQ_abs, Float *fenv_BWE)
{
  Short iL, ib, i, j, i8; 
  Short *ptrNbBand, *ptrBand, cnt;
  Float *ptr0, *ptr1, ftmp2, fenv_tmp;
  Float ftmp, fbuff_Fenv_BWE, fbuff_coef_pow;

  ptrNbBand = avqType;
  ptrBand = avqType + 3;
  for(iL=0; iL<2; iL++) 
  {
    for(ib= 0; ib<*ptrNbBand; ib++)
    {
      i = ptrBand[ib];
      i8 = i * 8;
      ptr0 = fcoef_SWB_AVQ + i8;
      ptr1 = fcoef_SWB_AVQ_abs + i8;
      /* calculate abs. value of coef_SWB_AVQ */
      cnt = 0;
      fenv_tmp = fenv_BWE[i] / 2.0f;
      fbuff_coef_pow = 0.0f;
      for(j=0; j<WIDTH_BAND; j++)
      {
        if( *ptr0 !=0) 
        {
          ftmp2 = abs_f(*ptr0);
          cnt += 1;
          *ptr1 = ftmp2 + fenv_tmp;
          ftmp2 = *ptr1;
          fbuff_coef_pow = fbuff_coef_pow + (ftmp2 * ftmp2);
        }
        ptr0++;
        ptr1++;
      }
      /* calculate buff_Fenv_BWE */
      ftmp = fenv_BWE[i] * fenv_BWE[i];
      ftmp *= 8;												
      ftmp -= fbuff_coef_pow;
      fbuff_Fenv_BWE = 0.0f;

      if (ftmp > 0.0f) 
		  fbuff_Fenv_BWE = ftmp * f_dentbl[cnt];
      fbuff_Fenv_BWE = Sqrt( fbuff_Fenv_BWE );
      /* calculate abs. value of coef_SWB_AVQ */
      ftmp2 = fbuff_Fenv_BWE;
      ptr0 -= WIDTH_BAND;
      ptr1 -= WIDTH_BAND;
      for(j=0; j<WIDTH_BAND; j++)
      {
        if (*ptr0== 0)
        {
          *ptr1 = ftmp2;
        }
        ptr0++;
        ptr1++;
      }
    }
    ptrNbBand++;
    ptrBand += 3;
  }
  ptrBand += 3;
  for(ib= 0; ib<*ptrNbBand; ib++)
  {
    i = ptrBand[ib];
    i8 = i * 8;
    ptr1 = fcoef_SWB_AVQ_abs + i8;
    zeroF(WIDTH_BAND, ptr1);
  }
  return;
}

static Short getBandLx_decodAVQ_flt(Short *smdct_coef_Lx, Float *fmdct_coef_AVQ, Short *bandTmp, Short nbBand, Short *bandLx, Short *bandZero)
{
  Short ib, cntLx, i;
  Short *ptr_Lx, *ptra, *ptrb, *ptrc;
  Float en, *ptr;
  
  ptr_Lx = smdct_coef_Lx;
  ptra = bandTmp;  
  ptrb = bandLx;
  ptrc = bandZero;
  cntLx = 0;
  for( ib=0; ib<nbBand; ib++ )
  {
    en = 0.0f;
    for (i=0; i<WIDTH_BAND; i++)
    {
        en += (Float)ptr_Lx[i] * (Float)ptr_Lx[i];
    }
    if( en == 0.0f )
    {
      *ptrc++ = *ptra++;
      ptr_Lx += WIDTH_BAND;
    }
    else
    {                                 
      cntLx = cntLx + 1;
      ptr = fmdct_coef_AVQ + (*ptra << 3);
      for (i=0; i<WIDTH_BAND; i++)
      {
        *ptr++ = (Float)(*ptr_Lx++);
      }
      *ptrb++ = *ptra++;
    }
  }

  return (cntLx);
}

static void s_sortIncrease(
                         Short n,         /* i  : array dimension */
                         Short nbMin,     /* i  : number of minima to sort */
                         Short *xin,    /* i  : arrray to be sorted */ 
                         Short *xout    /* o  : sorted array  */
                         )
{
  Short i, j, pos;
  Short xtmp[N_SV], xmin;

  for(i=0; i<n;i++)
  {
    xtmp[i] = xin[i];
  }
  for(i=0; i<nbMin; i++)
  {
    xmin  = xtmp[0];
    pos = 0;
    for(j=1; j<n; j++)
    {
      if( xtmp[j] < xmin )
      {
        pos = j;
      }
	  if( xtmp[j] <= xmin)
		  xmin = xtmp[j];
	  else
		  xmin = xmin;
    }
    xout[i] = xtmp[pos];
    xtmp[pos] = 32767;
  }
  return;
}

static void s_getIndexBitstream( Short nbBit, Short val, Short *nbBitCum, long *index)
{
  *nbBitCum += nbBit;
  *index = *index << nbBit;
  *index = *index + val;
  return;
}
