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

/*------------------------------------------------------------------------*
* Prototypes
*------------------------------------------------------------------------*/
static void decoder_coef_SWB_AVQ_adj( 
                                     const Short   zero_vector[],  /* o:  output vector signalising zero bands     */
                                     const Short   sord_b[],       /* i:  percept. importance  order of subbands   */
                                     Float   coef_SWB[],     /* i/o:  locally decoded MDCT coefs.             */
                                     unsigned short  *pBst,           /* i/o:  pointer to bitstream buffer            */
                                     Short  *unbits,
                                     const Short   smode,          /* 1: L1 / 2: L2                                */
                                     const Short   N_BITS_AVQ
                                     );

void f_if_negate(Float *fcoef_SWB, Float en)
{
  if(*fcoef_SWB < 0)
  {
    en = -en;
  }    
  *fcoef_SWB = en;

  return;
}

/* Constructor for AVQ decoder */
void* avq_decode_const (void)
{
  AVQ_state_dec *dec_st = NULL;

  dec_st = (AVQ_state_dec *) malloc (sizeof(AVQ_state_dec));
  if (dec_st == NULL) return NULL;

  avq_decode_reset ((void *)dec_st);

  return (void *)dec_st;
}

void avq_decode_dest (void *work)
{
  AVQ_state_dec *dec_st = (AVQ_state_dec *)work;

  if (dec_st != NULL)
  {
    free (dec_st);
  }
}

Short avq_decode_reset (void *work)
{
  AVQ_state_dec *dec_st = (AVQ_state_dec *) work;

  if (dec_st != NULL)
  {
    zeroS(sizeof(AVQ_state_dec)/2, work);

    dec_st->pre_cod_Mode = NORMAL;
    dec_st->pre_scoef_SWBQ0 = 15;
    dec_st->pre_scoef_SWBQ1 = 15;
  }
  return DECODER_OK;
}

void bwe_avq_buf_reset(void *work)
{   
  AVQ_state_dec *dec_st = (AVQ_state_dec *) work;

  zeroS(N_SV, dec_st->prev_zero_vector);

  zeroF(SWB_F_WIDTH, dec_st->fprefSp); 
  zeroF(SWB_F_WIDTH, dec_st->fbuffAVQ); 
  zeroF(SWB_F_WIDTH, dec_st->fpreAVQ0); 
  zeroF(SWB_F_WIDTH, dec_st->fpreAVQ1); 

  dec_st->pre_scoef_SWBQ0 = 15;
  dec_st->pre_scoef_SWBQ1 = 15;
  dec_st->pre_cod_Mode = NORMAL;

}

void decoder_SWBL1L2_AVQ( 
                         void*p_AVQ_state_dec, /* (i/o): Work space       */
                         unsigned short *pBst_L1,     /* i:	Input bitstream for SWBL1				*/
                         unsigned short *pBst_L2,     /* i:	Input bitstream for SWBL2				*/
                         const Short layers,   /* i:	number of SWB layers received			*/
                         const Float fenv_BWE[],  /* i:	Input normalized frequency envelope		*/
                         const Short *ord_bands,  /* i:	percept. importance	order of subbands   */
                         Short zero_vector[],  /* o:	Output vector signalising zero bands	*/
                         Float fcoef_SWB_AVQ[],/* o:	Output MDCT coefficients from AVQ		*/
                         const Short flg_bit,
                         Short *unbits_L1,
                         Short *unbits_L2
                         )
{
  Short i, j, flag;

  Short zero_vector_tmp[N_SV];    /* = 0 when zero subband, = 1 when L1 coeff. applied, = 2 when L2 coeff. applied */
  Short Nsv_L2;
  unsigned short *bptpt;
  Float en;
  Short smdct_coef_L1[WIDTH_BAND*N_SV_L1], smdct_coef_L2[WIDTH_BAND*N_SV_L2];
  Float fcoef_dec[SWB_F_WIDTH];

  Float fvec_base[N_SV*WIDTH_BAND];  
  Short cnt_used, cnt_unused, ind_corr_max;
  Float Gain;

  Short i8, k8;

  AVQ_state_dec *w_AVQ_state_dec = (AVQ_state_dec *)p_AVQ_state_dec;
  
  Short *sptr;
  Float *ptr;
  
  zeroS( N_SV, zero_vector_tmp );

  /* read and decode AVQ parameters from SWBL1 */
  *unbits_L1 = AVQ_demuxdec_bstr( pBst_L1, smdct_coef_L1, N_BITS_AVQ_L1, N_SV_L1 );

  if(flg_bit == 2)
  {
    *unbits_L1 = *unbits_L1 + 1;
  }
  
  /* find zero subbands */
  Nsv_L2 = 0;
  for( i = 0; i < N_SV_L1; i++ )
  {
    sptr = smdct_coef_L1 + (i << 3);
    en = 0.0f;
    for (j=0; j<WIDTH_BAND; j++)
    {
        en += *sptr * *sptr;
        sptr++;
    }
    if( en == 0.0f )
    {
      Nsv_L2 = Nsv_L2 + 1;
    }
    else
    {
      zero_vector_tmp[i] = 1;
    }
  }
  if( layers == 2 )
  {
    *unbits_L2 = AVQ_demuxdec_bstr( pBst_L2, smdct_coef_L2, N_BITS_AVQ_L2, N_SV_L2 );
  }
  /* reconstruct SWBL1 (and SWBL2) MDCT coefficients */
  k8 = 0;
  zeroF( SWB_F_WIDTH, fcoef_dec );
  for( i = 0; i<N_SV; i++ )
  {
    i8 = i << 3;

    if( (zero_vector_tmp[i] == 0) && layers == 2 )
    {
      if(k8 < N_SV_L2*8)
      {
        sptr = smdct_coef_L2 + k8;
        en = 0.0f;
        for (j=0; j<WIDTH_BAND; j++)
        {
            en += *sptr * *sptr;
            sptr++;
        }
        if( en > 0.0f )
        {
          zero_vector_tmp[i] = 2;
          for (j=0; j<WIDTH_BAND; j++)
          {
            fcoef_dec[i8+j] = smdct_coef_L2[k8+j];
          }
        }
      }
      k8 = k8 + 8;
    }
    else if(zero_vector_tmp[i] == 1)
    {
      for (j=0; j<WIDTH_BAND; j++)
      {
        fcoef_dec[i8+j] = smdct_coef_L1[i8+j];
      }
    }
  }
  i8 = flg_bit - 1;
  /* read detzer_flag */
  if( layers == 2 )
  {
    if( i8 != 0 && (*unbits_L1 > 0) )
    {
      if(flg_bit != 2)
      {
        bptpt = pBst_L1 + N_BITS_AVQ_L1 - *unbits_L1;
      }
      else
      {
        bptpt = pBst_L1 + N_BITS_AVQ_L1_PLS - *unbits_L1;
      }

      w_AVQ_state_dec->detzer_flg = GetBit( &bptpt, 1 );
      *unbits_L1 = *unbits_L1 - 1;
    }

    if( (w_AVQ_state_dec->detzer_flg > 0) && i8 == 0 )
    {
      w_AVQ_state_dec->detzer_flg = w_AVQ_state_dec->detzer_flg + 1;

      if(w_AVQ_state_dec->detzer_flg >= 5)
      {
        w_AVQ_state_dec->detzer_flg = 0;
      }
    }

    if( i8 != 0 && (w_AVQ_state_dec->detzer_flg > 0) )
    {
      w_AVQ_state_dec->detzer_flg = 1;
    }
  }
  else
  {
    w_AVQ_state_dec->detzer_flg = 0;
  }

  if( i8 != 0 && layers == 2 && (w_AVQ_state_dec->detzer_flg==0) )
  {
    ind_corr_max = 0;
    /* prepare vectors */
    cnt_unused = 0;
    cnt_used = 0;  
    for( i=0; i<N_SV; i++ )
    {
      if( zero_vector_tmp[i] == 0 )
      {
        cnt_unused = cnt_unused + 1;
      }
      if( zero_vector_tmp[i] != 0 )
      {
        i8 = i << 3;
        k8 = cnt_used << 3;
        movF(WIDTH_BAND, &fcoef_dec[i8], &fvec_base[k8]); 
        cnt_used = cnt_used + 1;
      }
    }

    flag = 0;	 /* tmp flag for L2 filling */
    /* reconstruct the zero subband 1 */
    if( (*unbits_L1 >= N_BITS_FILL_L1) && (cnt_used >= N_BASE_BANDS) )
    {
      /* read from the bitstream */
      if(flg_bit != 2)
      {
        bptpt = pBst_L1 + N_BITS_AVQ_L1 - *unbits_L1;
      }
      else
      {
        bptpt = pBst_L1 + N_BITS_AVQ_L1_PLS - *unbits_L1;
      }

      ind_corr_max = GetBit( &bptpt, N_BITS_FILL_L1 );
      *unbits_L1 = *unbits_L1 - N_BITS_FILL_L1;

      if(ind_corr_max < CORR_RANGE_L1)
      {
        for( i=0; i<N_SV; i++ )
        {
          if( zero_vector_tmp[i] == 0 )
          {
            ptr = &fvec_base[ind_corr_max];
            en = 0.0f;
            for (j=0; j<WIDTH_BAND; j++)
            {
                en += *ptr * *ptr;
                ptr++;
            }
            Gain = 1.0f / (Float)Sqrt(en / WIDTH_BAND); 
            i8 = i << 3;
            /* We Check for Gain < 1.0 because at 1.0 and Greater, we Set Gain=1.0 */
            if (Gain < 1.0f)
            {
              for (j=0; j<WIDTH_BAND; j++)
              {
              fcoef_dec[i8+j] = fvec_base[ind_corr_max+j] * Gain;
              }
            }
            else
            {
              movF(WIDTH_BAND, &fvec_base[ind_corr_max], &fcoef_dec[i8]);
            }
            zero_vector_tmp[i] = 2;
            break;
          }
        }
      }
      flag = ind_corr_max;	  /* tmp flag for L2 filling */
    }

    /* reconstruct the zero subband 2 */
    if(cnt_used >= N_BASE_BANDS && *unbits_L2 >= N_BITS_FILL_L2 && cnt_unused > 1 && (w_AVQ_state_dec->detzer_flg == 0) )
    {
      /* read from the bitstream */
      bptpt = pBst_L2 + N_BITS_AVQ_L2 - *unbits_L2;

      ind_corr_max = GetBit( &bptpt, N_BITS_FILL_L2 );
      *unbits_L2 = *unbits_L2 - N_BITS_FILL_L2;

      if(ind_corr_max < CORR_RANGE_L2)
      {
        for( i=0; i<N_SV; i++ )
        {
          if( zero_vector_tmp[i] == 0 )
          {
            if(flag < CORR_RANGE_L1)
            {
              ptr = &fvec_base[ind_corr_max];
              en = 0.0f;
              for (j=0; j<WIDTH_BAND; j++)
              {
                  en += *ptr * *ptr;
                  ptr++;
              }
              Gain = 1.0f / (Float)Sqrt(en / WIDTH_BAND); 
            
              i8 = i << 3;
              /* We Check for Gain < 1.0 because at 1.0 and Greater, we Set Gain=1.0 */
              if (Gain < 1.0f)
              {
                for (j=0; j<WIDTH_BAND; j++)
                {
					fcoef_dec[i8+j] = fvec_base[ind_corr_max+j] * Gain;
                }
              }
              else
              {
                movF(WIDTH_BAND, &fvec_base[ind_corr_max], &fcoef_dec[i8]);
              }
              zero_vector_tmp[i] = 2;
              break;
            }
            flag = 0;
          }
        }
      }
    }
  }
  /* backward reordering of subbands */
  for( i=0; i<N_SV; i++ )
  {
    zero_vector[ord_bands[i]] = zero_vector_tmp[i];
    movF( WIDTH_BAND, &fcoef_dec[i << 3], &fcoef_SWB_AVQ[ord_bands[i] << 3] );
  }
  /* denormalization per band */
  for( i=0; i<N_SV; i++ )
  {
    i8 = i << 3;
    for( j=0; j<WIDTH_BAND; j++ )
    {
      fcoef_SWB_AVQ[i8+j] = fcoef_SWB_AVQ[i8+j] * fenv_BWE[i];
    }
  }
  
  return;
}

/*--------------------------------------------------------------------------*
*  Function  swbl1_decode_AVQ()		                                         *
*  ~~~~~~~~~~~~~~~~~~~~~~~~~~~~                                            *
*  Main function for decoding Extension layers SWBL1 and SWBL2             *
*--------------------------------------------------------------------------*/
void swbl1_decode_AVQ (
                       void*  p_AVQ_state_dec,			/* (i/o): Work space										*/
                       unsigned short  *pBst_L1,		/* i:	Input bitstream for SWBL1                           */
                       unsigned short  *pBst_L2,		/* i:	Input bitstream for SWBL2                           */
                       const Float  *fEnv_BWE,			/* i:	Input frequency envelope from SWBL0				    */
                       Float *fcoef_SWB,				/* i/o:	Output SWB MDCT coefficients						*/
                       const Short   index_g_5bit,		/* i:	5 bit index of frame gain from SWBL0                */
                       const Short   cod_Mode,			/* i:	mode information from SWBL0                         */
                       const Short   layers				/* i:	number of swb layers received                       */
                       ) 
{
  int n;
  Short pos1;
  Short sord_b[N_SV];
  Short i8, sbit;
  Short flg_bit;
  Short zero_vector[N_SV];
  Short k;
  Short temp, pos2;
  Short i, j, index_gain;
  Short unbits_L1, unbits_L2;
  Float fFenv_BWE[N_SV];
  Float fip[N_SV];
  Float fenv_avrg[N_SV], fmin_coef;
  Float fenv_BWE[N_SV];
  Float fcoef_SWB_AVQ[SWB_F_WIDTH];
  Float fbuff_coef_pow, fbuff_Fenv_BWE, ftmp;
  Float ftmp16, ftmp16_2;
  Float f_k;
  Float f_en;
  Float *f_ptr0, *f_ptr1;

  unsigned short *bptpt;
  unsigned short *bptpt_L1, *bptpt_L2;

  Float fGainBWE   =  Pow( 2.0f , (Float)index_g_5bit );
  Float fGopt;
  Float f_temp     =  0.0f;

  AVQ_state_dec *w_AVQ_state_dec = (AVQ_state_dec *)p_AVQ_state_dec;

  zeroS(N_SV, zero_vector); 
  zeroS(N_SV, sord_b); 
  zeroF(N_SV, fip);
  zeroF(N_SV, fenv_BWE);
  zeroF(SWB_F_WIDTH, fcoef_SWB_AVQ);
  zeroS(N_SV, sord_b); 
  zeroF(N_SV, fFenv_BWE);
  zeroF(N_SV, fenv_avrg);

  /* calculate subband energy */
  f_loadSubbandEnergy ((Short)cod_Mode, (Float *)fEnv_BWE, fFenv_BWE , index_g_5bit);

  /* order subbands by decreasing perceptual importance  */
  movF(N_SV, fFenv_BWE, fip);

  f_Sort( fip, N_SV, sord_b, fenv_avrg );   /* senv_avrg used as tmp buffer only */

  for( i=0 ; i<N_SV ; i++ ){
    fenv_BWE[i] = fFenv_BWE[i];
  }

  flg_bit = 0;

  /* (Short)cod_Mode == NORMAL && w_AVQ_state_dec->pre_cod_Mode == NORMAL */
  if( ((Short)cod_Mode + w_AVQ_state_dec->pre_cod_Mode) == NORMAL )
  {   
    /*----------------------------------------------------------------------
    *
    * AVQ with mode switching
    *
    *---------------------------------------------------------------------*/

    /* get flg_bit from bitstream */
    /* -------------------------- */
    if( (*pBst_L1++) == ITU_G192_BIT_0 )
    {
      /* ---------------------------------------------------------------- */
      /* - decoding 0                                                     */
      /* ---------------------------------------------------------------- */
	  decoder_SWBL1L2_AVQ( (void*)w_AVQ_state_dec, pBst_L1, pBst_L2, layers, fenv_BWE, sord_b, zero_vector, fcoef_SWB_AVQ 
        , 0, &unbits_L1, &unbits_L2);
    }
	else
    {
      /* ----------------------------------------------------------------- */
      /* - decoding 1                                                      */
      /* ----------------------------------------------------------------- */
      Short cnt;  

	  Float ftmp_coef_SWB_AVQ[SWB_F_WIDTH];
      Float fcoef_SWB_AVQ_abs[SWB_F_WIDTH];  
      Float fenv_BWE_mod[N_SV];

	  zeroF( SWB_F_WIDTH, ftmp_coef_SWB_AVQ);
      zeroF( SWB_F_WIDTH, fcoef_SWB_AVQ_abs);
      zeroF( N_SV, fenv_BWE_mod);

      flg_bit = 1;

      /* calculate Fenv_BWE_mod */
      /* ---------------------- */
	  for( i=0 ; i<N_SV ; i++ ){
		fenv_BWE_mod[i] = fenv_BWE[i] * 0.600006103515625f;
	  }
	  decoder_SWBL1L2_AVQ( (void*)w_AVQ_state_dec, pBst_L1, pBst_L2, layers, fenv_BWE_mod, sord_b, zero_vector, fcoef_SWB_AVQ 
        , 1, &unbits_L1, &unbits_L2);

	  movF(SWB_F_WIDTH, fcoef_SWB_AVQ, ftmp_coef_SWB_AVQ);

      /* compute coef_SWB_AVQ */
      /* -------------------- */
      for(i=0; i<N_SV; i++)
      {
		fmin_coef = 8.0f;
        if( zero_vector[i] != 0 ) 
        {
		  i8 = i*8;

          /* calculate abs. value of coef_SWB_AVQ */
          cnt = 0;
		  fbuff_coef_pow = 0.0f;
		  fenv_avrg[i] = 8.0f;

          for(j=0; j<WIDTH_BAND; j++)
          {
            if(fcoef_SWB_AVQ[i8+j] != 0.0f)
            {
              ftmp = fcoef_SWB_AVQ[i8+j];
              if (ftmp < 0.0f) ftmp = -ftmp; //nagate(ftmp)

              if( ftmp < fenv_avrg[i] )
              {
                fenv_avrg[i] = ftmp;
              }
              fcoef_SWB_AVQ_abs[i8+j] = ftmp + (fenv_BWE[i]/2.0f);

              ftmp = fcoef_SWB_AVQ_abs[i8+j];
			  fbuff_coef_pow = fbuff_coef_pow + (ftmp * ftmp);
              cnt += 1;

              /* multiply sign inf. */
              ftmp = fcoef_SWB_AVQ_abs[i8+j];
              if( fcoef_SWB_AVQ[i8+j] < 0.0f )
              {
                ftmp = -ftmp;
              }
              fcoef_SWB_AVQ[i8+j] = ftmp;

              if( fcoef_SWB_AVQ_abs[i8+j] < fmin_coef )
              {
                fmin_coef = fcoef_SWB_AVQ_abs[i8+j];
              }
            }
		  }

          /* calculate buff_Fenv_BWE */
          ftmp = fenv_BWE[i] * fenv_BWE[i];
		  ftmp *= 8;
          ftmp = ftmp - fbuff_coef_pow;
          fbuff_Fenv_BWE = 0.0f;

          if( ftmp > 0 )
          {
            fbuff_Fenv_BWE = ftmp * f_dentbl[cnt];
			fbuff_Fenv_BWE = Sqrt( fbuff_Fenv_BWE );

            ftmp = fenv_BWE[i] * 0.5f;
			if( fbuff_Fenv_BWE <= ftmp)
			  fbuff_Fenv_BWE = fbuff_Fenv_BWE;
			else
			  fbuff_Fenv_BWE = ftmp;

            if( zero_vector[i] == 1 )
            {
              ftmp = fmin_coef * 0.125f;
              if( fbuff_Fenv_BWE > ftmp  && cnt == 1 )
              {
                fbuff_Fenv_BWE = ftmp;
              }
			  else if( fbuff_Fenv_BWE > (ftmp*2)  && cnt == 2 )
              {
                fbuff_Fenv_BWE = ftmp * 2.0f;
              }
			  else if( fbuff_Fenv_BWE > (ftmp*4) && cnt == 4 )
              {
                fbuff_Fenv_BWE = ftmp * 4.0f;
              }
            }
          }

		  /* calculate abs. value of coef_SWB_AVQ */
		  ftmp = fbuff_Fenv_BWE;
          for( j=0; j<WIDTH_BAND; j++ )
          {
            if( fcoef_SWB_AVQ[i8+j] == 0 )
            {
              ftmp16 = ftmp;
              if( fcoef_SWB[i8+j] < 0.0f )
              {
                ftmp16 = -ftmp16;
              }
              fcoef_SWB_AVQ[i8+j] = ftmp16;
            }
          }
        }/* end of if(zero_vector[i] != 0) */
		else
        {
          fenv_avrg[i] = fenv_BWE[i] * 1.79815673828125f;
        }
	  }

	  if( layers == 2 )
      {
        Short cnt_used, cnt_unused, ind_corr_max;
        Short tmp16, tmp16_2; 
        Short detprob_flg;

		Float f_iGain16,fden,fGain16;
		Float fvec_base[N_SV*WIDTH_BAND];
		Float f_tmp, f_en;

        /* read 'detprob_flg' from the L1 bitstream */
        detprob_flg = 0;
        bptpt = pBst_L1 + (N_BITS_AVQ_L1 - unbits_L1);
        if( unbits_L1 > N_BITS_FILL_L1+1 )
        {
          detprob_flg = GetBit( &bptpt, 2 );
          unbits_L1 = unbits_L1 - 2;
        }
		else if( unbits_L1 > N_BITS_FILL_L1 )
        {
          detprob_flg = GetBit( &bptpt, 1 );
          unbits_L1 = unbits_L1 - 1;
        }
		else if( unbits_L1 > 1 )
        {
          detprob_flg = GetBit( &bptpt, 2 );
          unbits_L1 = unbits_L1 - 2;
        }
		else if( unbits_L1 > 0 )
        {
          detprob_flg = GetBit( &bptpt, 1 );
          unbits_L1 = unbits_L1 - 1;
        }

		/* prepare vectors */
        cnt_unused = 0;
        cnt_used = 0;
        for( i=0; i<N_SV; i++ )
        {
          if( zero_vector[i] == 0 )
          {
            cnt_unused += 1;
            if( detprob_flg == 1 )
            {
              fenv_BWE[i] *= 0.5f;
            }
            if( detprob_flg == 2 )
            {
              fenv_BWE[i] *= 0.25;
            }
            if( detprob_flg == 3 )
            {
              fenv_BWE[i] *= 0.125f;
            }
          }
		  else
          {
            fden = fenv_BWE[i];

            f_iGain16 = 1.0f;
            if( 1.0f <= fden )
			{
              f_iGain16 = 1.0f / fden;
            }
            /*cnt_used*WIDTH_BAND*/
            tmp16 = cnt_used * 8;
            /*i*WIDTH_BAND*/
            tmp16_2 = i * 8;
            for( j=0; j<WIDTH_BAND; j++ )
            {
              fvec_base[tmp16+j] = fcoef_SWB_AVQ[tmp16_2+j] * f_iGain16;
            }
            cnt_used += 1;
          }
        }

		k = 0;			/* tmp flag for L2 filling */
        /* reconstruct the zero subband 1 */
        if( cnt_used >= N_BASE_BANDS && unbits_L1 >= N_BITS_FILL_L1 )
        {
          /* read from the bitstream */
          bptpt = pBst_L1 + (N_BITS_AVQ_L1 - unbits_L1);
          ind_corr_max = GetBit( &bptpt, N_BITS_FILL_L1 ); 
          unbits_L1 -= N_BITS_FILL_L1;

          if( ind_corr_max < CORR_RANGE_L1 )
          {
            for( i=0; i<N_SV; i++ )
            {
              if( zero_vector[i] == 0 )
              {
                /*correct_rat = 1/Sqrt(sum_vect_E( &vec_base[ind_corr_max], WIDTH_BAND )/WIDTH_BAND);*/
				  f_en = fvec_base[ind_corr_max] * fvec_base[ind_corr_max];
				  for( n=1 ; n<WIDTH_BAND ; n++ ){
					  f_en += fvec_base[ind_corr_max+n] * fvec_base[ind_corr_max+n];
				  }
                f_tmp = f_en;
                f_tmp = Sqrt( f_tmp );
                fGain16= f_tmp;

                if( fGain16 < 1.0f )
                {
                  fGain16 = fGain16 * fenv_BWE[i];
                }
				else
                {
                  fGain16 = fenv_BWE[i];
                }
                i8 = i*8;
                for( j=0; j<WIDTH_BAND; j++ )
                {
                  fcoef_SWB_AVQ[i8+j] = fvec_base[ind_corr_max+j] * fGain16;  
                }
                zero_vector[i] = 2;
                break;
              }
            }
          }
          k = ind_corr_max;			/* tmp flag for L2 filling */
        }

        if( cnt_used >= N_BASE_BANDS && unbits_L2 >= N_BITS_FILL_L2 && cnt_unused > 1 )
        {
          /* read from the bitstream */
          bptpt = pBst_L2 + (N_BITS_AVQ_L2 - unbits_L2);
          ind_corr_max = GetBit( &bptpt, N_BITS_FILL_L2 ); 
          unbits_L2 = (unbits_L2 - N_BITS_FILL_L2);

          if( ind_corr_max < CORR_RANGE_L2 )
          {
            for( i=0; i<N_SV; i++ )
            {
              if( zero_vector[i] == 0 )
              {
                if( k < CORR_RANGE_L1 )
                {
				  /*correct_rat = 1/Sqrt(sum_vect_E( &vec_base[ind_corr_max], WIDTH_BAND )/WIDTH_BAND);*/
				  f_en = fvec_base[ind_corr_max] * fvec_base[ind_corr_max];
				  for( n=1 ; n<WIDTH_BAND ; n++ ){
					  f_en += fvec_base[ind_corr_max+n] * fvec_base[ind_corr_max+n];
				  }
				  f_tmp = f_en;
                  f_tmp = Sqrt(f_tmp);
                  fGain16= f_tmp;

				  if( fGain16 < 1.0f )
                  {
                    fGain16 = fGain16 * fenv_BWE[i];
                  }
				  else
                  {
                    fGain16 = fenv_BWE[i];
                  }
                  i8 = i*8;
                  for( j=0; j<WIDTH_BAND; j++ )
                  {
                    fcoef_SWB_AVQ[i8+j] = fvec_base[ind_corr_max+j] * fGain16;
                  }
                  zero_vector[i] = 2;
                  break;
                }
                k = 0;
              }
            }
          }
        }
      }

      /*--------------------------------------------------------------------
      *
      * get sign information
      *
      *-------------------------------------------------------------------*/
	    /* set pointer adress */
        if( layers == 1 )
        {
          unbits_L1 -= (N_BITS_FILL_L1+2);
          if( unbits_L1 < 0 )
          {
            unbits_L1 = 0;
          }
        }
        bptpt_L1 = pBst_L1 + (N_BITS_AVQ_L1 - unbits_L1);

        if( layers == 2 )
        {
          bptpt_L2 = pBst_L2 + (N_BITS_AVQ_L2 - unbits_L2);
        }

        /* allocate sign information */
        for( i=0; i<N_SV; i++ )
        {
          if( zero_vector[i] == 1 )
          {
            i8 = i*8;
            for( j=0; j<WIDTH_BAND; j++ )
            {
              if( unbits_L1 > 0 )
              {
                if( ftmp_coef_SWB_AVQ[i8+j] == 0.0f )
                {
                  sbit = GetBit (&bptpt_L1, 1);

                  ftmp = abs_f(fcoef_SWB_AVQ[i8+j]);
                  if( sbit == 0 )
                  {
                    ftmp = -ftmp;
                  }
                  fcoef_SWB_AVQ[i8+j] = ftmp;
                  unbits_L1 -= 1;
                }
              }
			  else if( layers == 2 )
              {
                if( unbits_L2 > 0 )
                {
                  if( ftmp_coef_SWB_AVQ[i8+j] == 0 )
                  {
                    sbit = GetBit (&bptpt_L2, 1);

                    ftmp = abs_f(fcoef_SWB_AVQ[i8+j]);
                    if( sbit == 0 )
                    {
                      ftmp = -ftmp;
                    }
                    fcoef_SWB_AVQ[i8+j] = ftmp;
                    unbits_L2 -= 1;
                  }
                }
              }              
            }
          }
        }
      /*------------------------------------------------------------------*/
    }
    /* Read adjusted gain index */
    bptpt = pBst_L1 + N_BITS_AVQ_L1;
  }
  else
  {
    /*----------------------------------------------------------------------
    *
    * AVQ without mode switching
    *
    *---------------------------------------------------------------------*/

	decoder_SWBL1L2_AVQ( (void*)w_AVQ_state_dec, pBst_L1, pBst_L2, layers, fenv_BWE, sord_b, zero_vector, fcoef_SWB_AVQ 
      , 2, &unbits_L1, &unbits_L2);

    flg_bit = 2;
    /* Read adjusted gain index */
    bptpt = pBst_L1 + N_BITS_AVQ_L1_PLS;
  }
  index_gain = GetBit( &bptpt, N_BITS_GAIN_SWBL1 );

  /* Obtain adjusted gain */
  if( index_g_5bit == 0 && index_gain < 5 )
  {
    fGopt = f_sg0[index_gain];
  }
  else
  {
    fGopt = fgain_frac[index_gain] * fGainBWE;
  } 

  /* for zero subbands, keep MDCT coeficients from the BWE SWBL0 */
  for( i = 0; i<N_SV; i++ )
  {
    i8 = i*8;
    if( zero_vector[i] == 1 )
    {
      /* apply adjusted global gain to AVQ decoded MDCT coeficients */
      for( j = 0; j<WIDTH_BAND; j++ )
      {
        fcoef_SWB[i8+j] = fcoef_SWB_AVQ[i8+j] * fGopt;
      }
    }
	else if( zero_vector[i] == 2 )
    {
      /* apply global gain to AVQ decoded MDCT coefficients */
      for( j = 0; j<WIDTH_BAND; j++ )
      {
        fcoef_SWB[i8+j] = fcoef_SWB_AVQ[i8+j] * fGainBWE;
      }
    }
	else if( flg_bit == 1 )
    {
      /* apply global gain to AVQ decoded MDCT coefficients */
      for( j = 0; j<WIDTH_BAND; j++ )
      {
        ftmp16_2 = fenv_BWE[i] * fGainBWE;
        if( fcoef_SWB[i8+j] < 0 )
        {
          ftmp16_2 = -ftmp16_2;      
        }
        fcoef_SWB[i8+j] = ftmp16_2;
      } 
    }
	else if( w_AVQ_state_dec->detzer_flg == 1 )
    {
	  Float f_tmp,fGain;
	  Float Array_tmp;

	  Array_tmp = fcoef_SWB[i8] * fcoef_SWB[i8];
	  for( n=1 ; n<WIDTH_BAND ; n++ ){
		  Array_tmp += (fcoef_SWB[i8+n] * fcoef_SWB[i8+n]);
	  }
	  f_tmp = 1.0f + Array_tmp;
      ftmp16 = f_tmp;
      fGain = 1.0f;

      if( 1.0f <= ftmp16 )
      {
        fGain= fGainBWE / ftmp16;
      }

      f_tmp = Sqrt(fGain);
      f_tmp = f_tmp * 0.20001220703125f;
      fGain= f_tmp;
      for( j = 0; j<WIDTH_BAND; j++ )
      {
        if( fcoef_SWB[i8+j] < 0.0f )
        {
          fGain = -fGain;    
        }
        fcoef_SWB[i8+j] = fGain;
      }
    }
  }
  if( flg_bit != 1 && layers == 2 )
  {
    decoder_coef_SWB_AVQ_adj(zero_vector, sord_b, fcoef_SWB, pBst_L1, &unbits_L1, 1, N_BITS_AVQ_L1);

    /* modify decoded MDCT coefs. using gradient */
    decoder_coef_SWB_AVQ_adj(zero_vector, sord_b, fcoef_SWB, pBst_L2, &unbits_L2, 2, N_BITS_AVQ_L2);
  }

  for(i=0; i<N_SV; i++)
  {
    pos1 = i*8;
    pos2 = pos1 + WIDTH_BAND;
    f_k = fGopt;
    j = zero_vector[i]-1;
    if( j >= 0 )
    {
      if ( j > 0 ) /* sub(zero_vector[i], 1) > 0, so its value is 2 */
        f_k = fGainBWE;

      for( j = pos1; j < pos2; j++ )
      {
        w_AVQ_state_dec->fbuffAVQ[j] = fcoef_SWB_AVQ[j] * f_k;
      }
    } 
    fenv_avrg[i] = fenv_avrg[i] * f_k;
  }  
  if( (Short)cod_Mode == HARMONIC && w_AVQ_state_dec->pre_cod_Mode == HARMONIC )
  {
	Float f_preAVQ1[SWB_F_WIDTH];
    Float f_prefSp[SWB_F_WIDTH];
    Float f_preAVQ0[SWB_F_WIDTH];

    if( w_AVQ_state_dec->pre_scoef_SWBQ0 > w_AVQ_state_dec->pre_scoef_SWBQ1 )
    {
      temp = w_AVQ_state_dec->pre_scoef_SWBQ0 - w_AVQ_state_dec->pre_scoef_SWBQ1;

      for( i=0; i<SWB_F_WIDTH; i++ )
      {
		f_preAVQ1[i] = w_AVQ_state_dec->fpreAVQ1[i];
        f_prefSp[i] = w_AVQ_state_dec->fprefSp[i];
        f_preAVQ0[i] = w_AVQ_state_dec->fpreAVQ0[i];
      }
    }
	else
    {
      temp = w_AVQ_state_dec->pre_scoef_SWBQ1 - w_AVQ_state_dec->pre_scoef_SWBQ0;

      for(i=0; i<SWB_F_WIDTH; i++)
      {
		f_preAVQ1[i] = w_AVQ_state_dec->fpreAVQ1[i];
        f_prefSp[i] = w_AVQ_state_dec->fprefSp[i];
        f_preAVQ0[i] = w_AVQ_state_dec->fpreAVQ0[i];
      }
    }
    for( i = 0; i < N_SV; i++ )
    { 
      pos1 = i*8;
      if( w_AVQ_state_dec->prev_zero_vector[i] != 0 && zero_vector[i] == 0 ) 
      {
        for( j = 0; j < WIDTH_BAND; j++ )
        { 
          k = pos1 + j;
          fbuff_coef_pow = abs_f(fcoef_SWB[k]);
          if( fcoef_SWB_AVQ[k] == 0.0f )
          { 
            if( f_preAVQ0[k] > (f_preAVQ1[k] + f_prefSp[k]) )
            {
              f_temp = (f_preAVQ1[k] * 0.09375f) + (f_prefSp[k] * 0.09375f);
              f_temp += (fbuff_coef_pow * 0.0625f);
              f_en = f_temp;
            }
			else
            {
              f_temp = f_preAVQ0[k] + (f_preAVQ1[k] * 0.09375f);
              f_temp += (f_prefSp[k] * 0.09375f);
              f_temp += fbuff_coef_pow;
              f_en = f_temp;
            }
            f_if_negate( &fcoef_SWB[k], f_en);
          }
        }
      }
    }
  }
  else if( (Short)cod_Mode == TRANSIENT || w_AVQ_state_dec->pre_cod_Mode == TRANSIENT )
  {
	Float f_preAVQ1[SWB_F_WIDTH];

    for( i = 0; i < N_SV; i++ )
    {
      pos1 = i*8;
      if( zero_vector[i] != 0 )
      {
        for( j = 0; j < WIDTH_BAND; j++ )
        {
          k = pos1 + j;
          if( fcoef_SWB_AVQ[k] == 0 )
          {
            fbuff_coef_pow = abs_f(fcoef_SWB[k]);
            temp = 15;
            if( k == 0 )
            {			
              f_preAVQ1[k+1] = abs_f(w_AVQ_state_dec->fbuffAVQ[k+1]);
              f_temp = f_preAVQ1[k+1];
            }
			else if( k != (SWB_F_WIDTH - 1) )
            { 
              f_preAVQ1[k-1] = abs_f(w_AVQ_state_dec->fbuffAVQ[k-1]);	
              f_preAVQ1[k+1] = abs_f(w_AVQ_state_dec->fbuffAVQ[k+1]);
			  f_temp = f_preAVQ1[k-1];
              f_temp += f_preAVQ1[k+1];
              temp = 14;
            } 
            f_temp += fbuff_coef_pow;
            f_en = f_temp;
			ftmp16 = fenv_avrg[i] * 0.600006103515625f;
            f_en = f_min(f_en, ftmp16);
			  
            f_if_negate( &fcoef_SWB[k], f_en);
          }
        }
      }
    }  
  }
  else    /* NORMAL */
  {  
	Float f_abs_scoef_SWB[SWB_F_WIDTH];
    Float f_sh_sprefSp[SWB_F_WIDTH];

    if( (Short)flg_bit == 1 )
    {
      f_ptr0 = fcoef_SWB;
      f_ptr1 = f_abs_scoef_SWB;
	  for( j = 0; j < SWB_F_WIDTH; j++ )
      {
        *f_ptr1++ = abs_f(*f_ptr0++); 
      }

      f_ptr0 = w_AVQ_state_dec->fprefSp;
      f_ptr1 = f_sh_sprefSp;
	  for( n=0 ; n<SWB_F_WIDTH ; n++ ){
		  f_ptr1[n] = f_ptr0[n];
	  }

      for( i = 0; i < N_SV; i++ )
      {
        pos1 = i*8;
        if( zero_vector[i] == 0 )
        { 
          ftmp16 = fenv_avrg[i] * 0.600006103515625f;
          for( j = 0; j < WIDTH_BAND; j++ )
          {  
            k = pos1 + j;
            if( k == 0 )
            { 
              f_temp = (f_sh_sprefSp[k] * 0.70001220703125f) + (f_abs_scoef_SWB[k] * 0.899993896484375f);
              f_temp += (f_sh_sprefSp[k+1] * 0.20001220703125f);
              f_temp += (f_abs_scoef_SWB[k+1] * 0.20001220703125f);
            }
			else if( k == (N_SV * WIDTH_BAND - 1) )
            {
              f_temp = (f_sh_sprefSp[k] * 0.5f) + (f_abs_scoef_SWB[k] * 0.70001220703125f);
              f_temp += (f_sh_sprefSp[k-1] * 0.29998779296875f);
              f_temp += (abs_f(fcoef_SWB[k-1]) * 0.5f);
            }
			else
            { 
              f_temp = (f_sh_sprefSp[k] * 0.5f) + (f_abs_scoef_SWB[k] * 0.70001220703125f);
              f_temp += (f_sh_sprefSp[k-1] * 0.20001220703125f);
              f_temp += (abs_f(fcoef_SWB[k-1]) * 0.20001220703125f);
              f_temp += (f_sh_sprefSp[k+1] * 0.20001220703125f);
              f_temp += (f_abs_scoef_SWB[k+1] * 0.20001220703125f);
            } 
            f_en = f_temp; 
            f_en = f_min(f_en, ftmp16);  
            f_if_negate( &fcoef_SWB[k], f_en);
          }  
        }
		else
        {
          ftmp16 = fenv_avrg[i] * 0.600006103515625f;
          for( j = 0; j < WIDTH_BAND; j++ )
          {
            k = pos1 + j;
            if( w_AVQ_state_dec->fbuffAVQ[k] == 0 )
            {
              if( k == 0 )
              {
                f_temp = (f_sh_sprefSp[k] * 0.149993896484375f) + (f_abs_scoef_SWB[k] * 0.649993896484375f);
                f_temp += (f_sh_sprefSp[k+1] * 0.100006103515625f);
                f_temp += (f_abs_scoef_SWB[k+1] * 0.100006103515625f);
              }
			  else if( k == (N_SV * WIDTH_BAND - 1) )
              {
                f_temp = (f_sh_sprefSp[k] * 0.149993896484375f) + (f_abs_scoef_SWB[k] * 0.649993896484375f);
                f_temp += (f_sh_sprefSp[k-1] * 0.04998779296875f);
                f_temp += (fcoef_SWB[k-1] * 0.149993896484375f);
              }
			  else
              {
                f_temp = (f_sh_sprefSp[k] * 0.149993896484375f) + (f_abs_scoef_SWB[k] * 0.649993896484375f);
                f_temp += (f_sh_sprefSp[k-1] * 0.04998779296875f);
                f_temp += (abs_f(fcoef_SWB[k-1]) * 0.04998779296875f);
                f_temp += (f_sh_sprefSp[k+1] * 0.04998779296875f);

                f_temp += (f_abs_scoef_SWB[k+1] * 0.04998779296875f);
              }
              f_en = f_temp;
              f_en = f_min(f_en, ftmp16);         
              f_if_negate( &fcoef_SWB[k], f_en);
            }
			else
            {
              fcoef_SWB[k] = fcoef_SWB[k];
            }
          }
        }
      }
    } 
  }
  movSS(N_SV, zero_vector, w_AVQ_state_dec->prev_zero_vector);
  movF(SWB_F_WIDTH, w_AVQ_state_dec->fpreAVQ1, w_AVQ_state_dec->fpreAVQ0);
  for( n=0 ; n<SWB_F_WIDTH ; n++ ){
	  w_AVQ_state_dec->fpreAVQ1[n] = abs_f( w_AVQ_state_dec->fbuffAVQ[n] );
	  w_AVQ_state_dec->fprefSp[n] = abs_f( fcoef_SWB[n] );
  }
  w_AVQ_state_dec->pre_scoef_SWBQ0 = w_AVQ_state_dec->pre_scoef_SWBQ1;

  if( flg_bit == 1 )
  {
    /* ------------------------------------------------------------------ */
    /* smoothing (if decoding mode is 1)                                  */
    /* ------------------------------------------------------------------ */

	Float fbuff_abs;
    Float fcoef_tmp;

    for( i = 0; i<N_SV; i++ )
    {
      i8 = i*8;
      for( j = 0; j<WIDTH_BAND; j++ )
      {
        fbuff_abs = abs_f(fcoef_SWB[i8+j]);
        fcoef_tmp = (0.850006103515625f * fbuff_abs) + (0.149993896484375f * w_AVQ_state_dec->fcoef_SWB_abs_old[i8+j]);

        if( fcoef_SWB[i8+j] < 0 )
        {
          fcoef_tmp = -fcoef_tmp;
        }
        if( w_AVQ_state_dec->fcoef_SWB_abs_old[i8+j] != 0 )
        {
          fcoef_SWB[i8+j] = fcoef_tmp;
        }
        w_AVQ_state_dec->fcoef_SWB_abs_old[i8+j] = fbuff_abs;
      }
    }
  }
  else
  {
    /* initialize */
    zeroF(SWB_F_WIDTH, w_AVQ_state_dec->fcoef_SWB_abs_old);

  }
  w_AVQ_state_dec->pre_cod_Mode = cod_Mode;

  return;
}

/*-----------------------------------------------------------------*
*   Funtion  decoder_coef_SWB_AVQ_adj                             *
*            ~~~~~~~~~~~~~~~~~~~~~~~~                             *
*   calculate gradient of regression line                         *
*-----------------------------------------------------------------*/
static void decoder_coef_SWB_AVQ_adj( 
                                     const Short   zero_vector[],  /* o:  output vector signalising zero bands     */
                                     const Short   sord_b[],       /* i:  percept. importance  order of subbands   */
                                     Float   coef_SWB[],     /* i/o:  locally decoded MDCT coefs.             */
                                     unsigned short  *pBst,           /* i/o:  pointer to bitstream buffer            */
                                     Short  *unbits,
                                     const Short   smode,          /* 1: L1 / 2: L2                                */
                                     const Short   N_BITS_AVQ
                                     )
{
  Short i, j, b, n;
  Short idx, max_idx, bit_alloc;
  Short i8, id8;

  /* pointer of bitstream */
  pBst = pBst + N_BITS_AVQ - (*unbits);

  /* calculate the number of vector */
  n = 0;
  for(i=0; i<N_SV; i++)
  {
    if( zero_vector[i] == smode )
    {
      n += 1;
    }
  }

  /* calculate gradient */
  /* ------------------ */
  for( i=0; i<N_SV; i++ )
  {
    /* check unbits */
    if (!*unbits)
    {
      break;
    }

    /* calculate gradient of each vector */
    b = sord_b[i];
    if( zero_vector[b] == smode ) 
    {
      /* calculate bit allocation */
      max_idx = 1;
      bit_alloc = 1;
      if( *unbits > n )
      {
        max_idx = 3;
        bit_alloc = 2;
      }

      n -= 1;

      /* read from the bitstream */                    
      idx = GetBit(&pBst, bit_alloc);
      *unbits -= bit_alloc;

      /* update locally decoded MDCT coefs. */
      if( idx )
      { 
        i8 = b*8; 
        id8 = (idx-1)*8;

        for( j=0; j<WIDTH_BAND; j++ )
        {
          coef_SWB[i8+j] = coef_SWB[i8+j] * fgrad[id8+j]; 
        }

      }     
    } /* if (zero_vector[b] == mode) */
  } /* for (i=0; i<N_SV; i++) */
}
