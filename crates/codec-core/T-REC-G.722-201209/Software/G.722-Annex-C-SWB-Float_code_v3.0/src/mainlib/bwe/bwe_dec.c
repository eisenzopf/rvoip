/* ITU G.722 3rd Edition (2012-09) */

/*--------------------------------------------------------------------------
 ITU-T G.722 Annex C (ex G.722-SWB-Float) Source Code
 Software Release 1.01 (2012-07)
 (C) 2012 France Telecom, Huawei Technologies, VoiceAge Corp., NTT.
--------------------------------------------------------------------------*/

#include "bit_op.h"
#include "bwe_mdct.h"
#include "bwe.h"
#include "softbit.h"
#include "table.h"

#include <math.h>

#define DECODER_OK  2
#define DECODER_NG  3

void* bwe_decode_const(void)
{
  BWE_state_dec  *dec_st = NULL;

  dec_st = (BWE_state_dec *)malloc( sizeof(BWE_state_dec) );
  if (dec_st == NULL) return NULL;

  bwe_decode_reset( (void *)dec_st );

  return (void *)dec_st;
}

void bwe_decode_dest( void *work ) 
{
  BWE_state_dec  *dec_st=(BWE_state_dec *)work;

  if (dec_st != NULL)
  {
    free( dec_st );
  }
}

Short bwe_decode_reset( void *work ) 
{
	BWE_state_dec  *dec_st=(BWE_state_dec *)work;
	Short i = 0;

	if (dec_st != NULL) 
	{
		dec_st->pre_tEnv = 0.0f;
		zeroF(SWB_T_WIDTH, dec_st->fpre_wb);
		zeroF(10, dec_st->pre_fEnv);
		zeroF(HALF_SUB_SWB_T_WIDTH, dec_st->tPre);
		zeroF(L_FRAME_WB, dec_st->fPrev);
		zeroF(L_FRAME_WB, dec_st->fCurSave);
		zeroF(L_FRAME_WB, dec_st->fPrev_wb);
		zeroF(L_FRAME_WB, dec_st->fCurSave_wb);
		dec_st->norm_pre = 0;
		dec_st->norm_pre_wb = 0;
		dec_st->pre_mode = 0;
		dec_st->attenu2 = 0.1f;
		dec_st ->prev_enerL = 0.0f;
		zeroF(WB_POSTPROCESS_WIDTH, dec_st->spGain_sm);
		dec_st->modeCount = 0; 
		dec_st->Seed = 21211L;	
	}
	return DECODER_OK;
}

Short bwe_dec_update(                              /*to maintain mid-band post-processing memories up to date in case of WB frame*/
					  Float  		 *fy_low,    	   /* (i): Input lower-band WB signal */
					  void           *work             /* (i/o): Pointer to work space        */
					  )
{

	Float fPrev[L_FRAME_WB], fY[L_FRAME_WB];

	BWE_state_dec *dec_st = (BWE_state_dec *) work;

 	movF(L_FRAME_WB, dec_st->fpre_wb, fPrev);
	movF(L_FRAME_WB, fy_low, dec_st->fpre_wb);

	f_bwe_mdct (fPrev, fy_low, fY, 1);
	f_PCMSWB_TDAC_inv_mdct (fy_low, fY, dec_st->fPrev_wb, 
		 0, dec_st->fCurSave_wb);	


	return(0);
}




void MaskingFreqPostprocess( /* Generating the frequency gains with masking effect */
             Float *pFreq,   /* (i) : Frequency coefficients */
             int Len,        /* (i) : Length of frequency coefficients */
             Float *gain,    /* (o) : Gains for each frequency */
             Float Control   /* (i) : Control the degree of postprocessing : 0<Control<1 ;
                                      Control=0 means no postprocessing */
             )
{
    Float fw1, fw2, w, sum_w;
    Float AverageMag, Menergy, Menergy_th;
    Float Fq_abs[L_FRAME_WB - ZERO_SWB], Norm, Max_g;
    int i, j;
    Short j1, j2;

    /* Calc average magnitude */
    AverageMag = EPS; /* initial */
    for (i = 0; i < Len; i++) 
    {
        Fq_abs[i] = (Float) Fabs(pFreq[i]); /* implement: save Fabs(pFreq[i]) to be used later */
        AverageMag += Fq_abs[i];
    }
    AverageMag /= Len;

    for (i = 0; i < Len; i++)
    {
        /* Estimate Masked magnitude */
        j1 = PST_J1[i];
        j2 = PST_J2[i];
        fw1 = PST_FW1[i];
        fw2 = PST_FW2[i];
        sum_w = PST_SUMW1[i];
        w = 1;
        Menergy = 0; /* Menergy is masked magnitude */
        for (j = 0; j < j1; j++)
        {
            Menergy += w * Fq_abs[i - j]; 
            w -= fw1;
        }

        w = 1;
        for (j = 1; j < j2; j++) 
        {
            w -= fw2;
            Menergy += w * Fq_abs[i + j];
        }
        Menergy *= sum_w;

        /* Estimate Masking threshold */

        j1 = PST_J11[i];
        j2 = PST_J22[i];
        fw1 = PST_FW11[i];
        fw2 = PST_FW22[i];
        sum_w = PST_SUMW2[i];
        w = 1;
        Menergy_th = 0.1f; /* Menergy_th is masking threshold */
        for (j = 0; j < j1; j++) 
        {
            Menergy_th += w * Fq_abs[i - j];
            w -= fw1;
        }
        w = 1;
        for (j = 1; j < j2; j++) 
        {
            w -= fw2;
            Menergy_th += w * Fq_abs[i + j];
        }

        Menergy_th *= sum_w;
        Menergy_th = 0.75f * Menergy_th + 0.25f * AverageMag; /* add over-all masking threshold */

        /*    Estimate gains  */
        gain[i] = Menergy / Menergy_th;
    }

    /* Norm gains */
    Menergy = EPS; 
    Max_g = 0;
    for (i = 0; i < Len; i++) 
    {
        Menergy += gain[i] * Fq_abs[i];
        if (gain[i] > Max_g)
        {
            Max_g = gain[i];
        }
    }
    Menergy /= Len;
    Norm = AverageMag / Menergy;

    if (Max_g * Norm > 1.5f) 
    {
        Norm *= 1.5f / (Max_g * Norm);
    }
    if (AverageMag < 32) 
    {
        Norm *= Control * AverageMag / 32 + (1 - Control);
    }
    for (i = 0; i < Len; i++) 
    {
        gain[i] *= Norm;
    }

    return;
}

void IF_Coef_Generator(
				 Float *MDCT_wb,           //(i)input lower band MDCTs
		         Short mode,               //(i)frame BWE mode
		         Float *fSpectrum,         //(o)SWB BWE MDCT coefficients
			     Float *fEnv,              //(i)Frequency envelops
			     Float fGain,
			     Float *pre_fEnv,
			     Short pre_mode,
			     Short noise_flag,
			     Short bit_switch_flag,
			     Short s_modeCount,
			     Short *Seed 
		  )
{
	int i, j;
	Float norm[SWB_NORMAL_FENV], noise[SWB_F_WIDTH];
	Float *pit, *pit1;
	Float weight;
	Float f_Env;

	for(i = 0; i<SWB_F_WIDTH; i ++) 
	{
		*Seed = (Short)(12345*(*Seed) + 20101);
		noise[i] = (Float)(*Seed)/32768;
	}
	if (noise_flag == 1 && pre_mode != HARMONIC)
	{
		for (i=0; i<SWB_F_WIDTH; i++)
		{
			MDCT_wb[i] = noise[i];
		}
	}
	else if (mode == NORMAL && pre_mode == NORMAL && s_modeCount == 0)
	{
		for (i=0; i<SWB_F_WIDTH_HALF; i++)
		{
			MDCT_wb[i] = MDCT_wb[SWB_F_WIDTH_HALF+i];
		}
	}

	pit = MDCT_wb;
	pit1 = fSpectrum;
	if (mode == TRANSIENT)
	{		
		for (i = 0; i < SWB_TRANSI_FENV; i++)
		{
			norm[i] = EPS;
			for (j = 0; j < SWB_TRANSI_FENV_WIDTH; j++)
			{
				norm[i] += (*pit) * (*pit);
				pit++;
			}
			norm[i] = Sqrt( SWB_TRANSI_FENV_WIDTH / norm[i] );

			if (fEnv[i] == 0)
			{
				fEnv[i] = (Float) 0.02 * fGain;
			}

			pit -= SWB_TRANSI_FENV_WIDTH;
			for (j = 0; j < SWB_TRANSI_FENV_WIDTH; j++)
			{
				*pit1 = *pit * norm[i] * fEnv[i];
				pit++;
				pit1++;
			}
		}
	}
	else
	{
		weight = (Float) 0.5;
		if (mode == NORMAL)
		{
			weight = (Float) 0.7;
		}

		if(bit_switch_flag == 0 || bit_switch_flag == 2)
		{
			for (i = 0; i < SWB_NORMAL_FENV; i++)
			{
				norm[i] = EPS;
				for (j = 0; j < FENV_WIDTH; j++)
				{
					norm[i] += (*pit) * (*pit);
					pit++;
				}

				norm[i] = Sqrt( FENV_WIDTH / norm[i] );

				f_Env = fEnv[i];
				if ((Fabs( fEnv[i] - pre_fEnv[i] ) <= (3.0 * fGain)) && (pre_mode != TRANSIENT) )
				{
					f_Env = weight * fEnv[i] + (1.0f - weight) * pre_fEnv[i];
				}

				pit -= FENV_WIDTH;
				f_Env *= norm[i];
				for (j = 0; j < FENV_WIDTH; j++)
				{
					*pit1 = *pit * f_Env;
					pit++;
					pit1++;
				}
			}
		}
		else
		{
			for (i = 0; i < SWB_NORMAL_FENV; i++)
			{
				norm[i] = EPS;
				for (j = 0; j < FENV_WIDTH; j++)
				{
					norm[i] += (*pit) * (*pit);
					pit++;
				}

				norm[i] = Sqrt( FENV_WIDTH / norm[i] );
				f_Env = 0.5f * (fEnv[i]+pre_fEnv[i]);

				pit -= FENV_WIDTH;
				f_Env *= norm[i];
				for (j = 0; j < FENV_WIDTH; j++)
				{
					*pit1 = *pit * f_Env;
					pit++;
					pit1++;
				}
			}
		}
	}

	return;
}

Short bwe_dec_freqcoef(
					    unsigned short **pBit,       /* (i): Input bitstream                */
                        Float  *fy_low,    	         /* (i): Input lower-band WB signal */
                        void    *work,               /* (i/o): Pointer to work space        */
                        Short  *sig_Mode,
                        Float   *f_sTenv_SWB,        /* (o) */ 
                        Float   *f_scoef_SWB,
                        Short  *index_g,
                        Float   *f_sFenv_SVQ,        /* (o): decoded spectral envelope with no postprocess. */
                        Short     ploss_status,
                        Short  bit_switch_flag,
                        Short  prev_bit_switch_flag
						)
{
	BWE_state_dec *dec_st = (BWE_state_dec *) work;
	Short i, j, mode;
	Short index_fGain = 0;
	Short index_fEnv[SWB_TRANSI_FENV];
	Short index_fEnv_codebook[NUM_FENV_CODEBOOK];
	Short index_fEnv_codeword[NUM_FENV_VECT];
	Short noise_flag = 0;
	Short T_modify_flag = 0;
	Short temp;

	Float MDCT_wb[L_FRAME_WB];
	Float f_sfEnv[SWB_NORMAL_FENV], f_sNoExpand_fGain; 
	Float f_sfGain;
	Float f_senerL, f_senerH, f_senerL1;
	Float *f_spit_fen = fcodebookL;
	Float f_i_max, f_i_min,f_i_avrg;
	Float *f_spMDCT_wb;
	Float f_senerWB;
	Float f_spGain[36], f_sMDCT_wb_postprocess[L_FRAME_WB];
	Float fY[L_FRAME_WB];

	zeroF( L_FRAME_WB, fY );
	zeroF( SWB_NORMAL_FENV, f_sfEnv );
	zeroF( L_FRAME_WB, MDCT_wb );

	if ( ploss_status == 1 )
	{
		/* MDCT on 80 samples in the 0-8kHz band */
		f_bwe_mdct (dec_st->fpre_wb, fy_low, fY, 1);
		movF (80, fY, MDCT_wb);

		for (i=0; i<L_FRAME_WB; i++) 
		{
			MDCT_wb[i] *= 16.0f * Sqrt(5.0f);
		}

		f_spMDCT_wb = &MDCT_wb[L_FRAME_WB - WB_POSTPROCESS_WIDTH];
		MaskingFreqPostprocess(f_spMDCT_wb, WB_POSTPROCESS_WIDTH, f_spGain, 0.5f);

		if (dec_st->pre_mode == TRANSIENT)
		{
			movF( WB_POSTPROCESS_WIDTH, f_spGain, dec_st->spGain_sm );
		}
		for (i = 0; i < WB_POSTPROCESS_WIDTH; i++) 
		{
			dec_st->spGain_sm[i] = 0.5f*(dec_st->spGain_sm[i] + f_spGain[i]);

			if (dec_st->spGain_sm[i] < 1.15f)
			{
				f_spMDCT_wb[i] *= dec_st->spGain_sm[i];
			}
		}   
   
		for (i = 0; i < L_FRAME_WB; i++) 
		{
			f_sMDCT_wb_postprocess[i] = MDCT_wb[i] / (16.0f * Sqrt(5.0f));
		}

		f_PCMSWB_TDAC_inv_mdct(fy_low,f_sMDCT_wb_postprocess,dec_st->fPrev_wb, 0,dec_st->fCurSave_wb);

		*sig_Mode = NORMAL;
		for (i=0; i<SWB_NORMAL_FENV; i++)
		{
			dec_st->pre_fEnv[i] *= 0.85f;
		}

		return (0);
	}
	else
	{
		/* MDCT on 80 samples in the 0-8kHz band */
		f_bwe_mdct (dec_st->fpre_wb, fy_low, fY, 1);
		movF (80, fY, MDCT_wb);

		for (i=0; i<L_FRAME_WB; i++) 
		{
			MDCT_wb[i] *= 16.0f * Sqrt(5.0f);
		}	
		
		f_senerL = 0.0f;
		for(i=0; i<ENERGY_WB; i++)
		{
			f_senerL += MDCT_wb[i] * MDCT_wb[i];
		}
		f_senerL = Sqrt(f_senerL/ENERGY_WB);

		if(bit_switch_flag == 0 || bit_switch_flag == 2) /* test if bit_switch_flag == 0 or 2 */
		{
			mode = GetBit(pBit, 2);			
			index_fGain = GetBit(pBit, 5);
			f_sfGain = (Float)(1 << index_fGain); 

			if(mode == TRANSIENT)  
			{
				dec_st->modeCount = 0;
				f_sNoExpand_fGain = f_sfGain * INV_TRANSI_FENV_EXPAND;

				for (i=0;i<VQ_FENV_DIM;i++)
				{
					index_fEnv[i] = GetBit(pBit, 4);
					f_sfEnv[i] = index_fEnv[i] * f_sNoExpand_fGain;
				}

				for (i=0; i<SWB_NORMAL_FENV; i++)
				{
					f_sFenv_SVQ[i] = f_sfEnv[i];
				}
			}
			else
			{
				index_fEnv_codebook[0] = GetBit(pBit, 1);	
				index_fEnv_codebook[1] = GetBit(pBit, 1);	
				index_fEnv_codeword[0] = GetBit(pBit, 6);	
				index_fEnv_codeword[1] = GetBit(pBit, 6);

				f_spit_fen = fcodebookH;
				if (index_fEnv_codebook[0] == 0)
				{
				    f_spit_fen = fcodebookL;
				}

				temp = index_fEnv_codeword[0] << 2;
				movF(SWB_NORMAL_FENV_HALF, &f_spit_fen[temp], f_sfEnv);				

				f_spit_fen = fcodebookH;
				if (index_fEnv_codebook[1] == 0)
				{
				  f_spit_fen = fcodebookL;
				}

				temp = index_fEnv_codeword[1] << 2;
				movF(SWB_NORMAL_FENV_HALF, &f_spit_fen[temp], &f_sfEnv[SWB_NORMAL_FENV_HALF]);

				for(i=0; i<SWB_NORMAL_FENV; i++)
				{
					f_sfEnv[i] *= f_sfGain;
				}

				movF(SWB_NORMAL_FENV, f_sfEnv, f_sFenv_SVQ);

				if(mode == HARMONIC)
				{								
					dec_st->modeCount += 1;
				}
				else
				{								
					if(dec_st->modeCount > 0)
					{
						dec_st->modeCount -= 1;
					}

					noise_flag = 1; 
					if(mode == 0)
					{
						noise_flag = 0; 
					}
					mode = NORMAL;					

					f_i_max = 0.0f;
					f_i_min = 10.0f;
					for(i=0; i<SWB_NORMAL_FENV; i++)
					{
						if(f_sfEnv[i] > f_i_max)
						{
							f_i_max = f_sfEnv[i];
						}
						if(f_sfEnv[i] < f_i_min)
						{
							f_i_min = f_sfEnv[i];
						}
					}

					f_i_avrg = 0.0f;
					for (i = 0; i < SWB_NORMAL_FENV; i++)
					{
						f_i_avrg += f_sfEnv[i];
					}
					f_i_avrg *= (Float) 0.125;

					if(((f_i_max - f_i_min) > 2.5f) && (f_i_min < 12.0f))
					{
						for(i=0; i<SWB_NORMAL_FENV; i++)
						{
							if(f_sfEnv[i] < 0.4f*f_i_avrg)
							{
								f_sfEnv[i] *= (Float) 0.5;
							}
						}
					}
				}
			}
		}
		else              
		{
			f_spMDCT_wb = &MDCT_wb[20];			
			f_sfEnv[0] = EPS;
			for(i=0; i<4; i++)
			{
				f_sfEnv[0] += (*f_spMDCT_wb) * (*f_spMDCT_wb);
				f_spMDCT_wb++;
			}
			f_sfEnv[0] = Sqrt(f_sfEnv[0]/8);

			f_senerWB = 0.0f;
			f_spMDCT_wb = &MDCT_wb[24];
			for(i=1; i<8; i++)
			{
				f_sfEnv[i] = EPS;
				for(j=0; j<8; j++)
				{
					f_sfEnv[i] += (*f_spMDCT_wb) * (*f_spMDCT_wb);
					f_spMDCT_wb++;
				}
				if(i > 2)
				{
					f_senerWB += f_sfEnv[i];
				}
				f_sfEnv[i] = Sqrt(f_sfEnv[i]/8);
			}
			f_senerWB = Sqrt(f_senerWB/40);

			f_i_max = f_sfEnv[0];
			for(i=1; i<8; i++)
			{
				if(f_sfEnv[i] > f_i_max)
				{
					f_i_max = f_sfEnv[i];
				}
			}

			if(prev_bit_switch_flag == 0)
			{
				dec_st->attenu2 = 0.1f;
			}

			if(dec_st->attenu2 < 0.5)
			{
				if(dec_st->prev_enerL > 0.5f*f_senerL && dec_st->prev_enerL < 2.0f*f_senerL)
				{
					for(i=0; i<8; i++)
					{
						f_sfEnv[i] *= 0.125f*(f_senerWB/f_i_max);
						f_sfEnv[i] = dec_st->pre_fEnv[i]*(1.0f-dec_st->attenu2) + dec_st->attenu2*f_sfEnv[i];
					}
				}
				else
				{
					for(i=0; i<8; i++)
					{
						f_sfEnv[i] *= 0.125f*(f_senerWB/f_i_max);
						f_sfEnv[i] = 0.5f*(dec_st->pre_fEnv[i] + f_sfEnv[i]);
					}
				}
				dec_st->attenu2 += 0.01f;
			}
			else
			{
				for(i=0; i<8; i++)
				{
					f_sfEnv[i] *= 0.125f*(f_senerWB/f_i_max);
					f_sfEnv[i] = 0.5f*(dec_st->pre_fEnv[i] + f_sfEnv[i]);
				}
			}

			mode = NORMAL;
			movF(SWB_NORMAL_FENV, f_sfEnv, f_sFenv_SVQ);
		}

		f_senerL1 = (Float) 0.0; 
		for (i = 0; i < L_FRAME_WB; i++)
		{
			f_senerL1 += MDCT_wb[i] * MDCT_wb[i];
		}
		if(f_senerL1 > 32767)
		{
			i=0;
		}
		f_senerL1 = (Float) Sqrt(f_senerL1 / L_FRAME_WB);

		f_senerH = (Float) 0.0;
		for (i = 0; i < SWB_NORMAL_FENV; i++)
		{
			f_senerH += f_sfEnv[i];
		}
		f_senerH /= SWB_NORMAL_FENV;

		f_spMDCT_wb = &MDCT_wb[L_FRAME_WB - WB_POSTPROCESS_WIDTH];

		if ((mode != TRANSIENT) && (f_senerL > 0.8f*f_senerH))
		{
			MaskingFreqPostprocess(f_spMDCT_wb, WB_POSTPROCESS_WIDTH, f_spGain, 0.5f);

			if (dec_st->pre_mode == TRANSIENT)
			{
				movF(WB_POSTPROCESS_WIDTH, f_spGain, dec_st->spGain_sm);
			}
			for (i = 0; i < WB_POSTPROCESS_WIDTH; i++) 
			{
				dec_st->spGain_sm[i] = 0.5f * dec_st->spGain_sm[i] + 0.5f * f_spGain[i];
				if (dec_st->spGain_sm[i] < 1.15f)
				{
					f_spMDCT_wb[i] *= dec_st->spGain_sm[i];
				}
			}
		}
		else
		{
			for (i = 0; i < WB_POSTPROCESS_WIDTH; i++) 
			{
				dec_st->spGain_sm[i] = 1.0f;
			}
		}

		for (i = 0; i < L_FRAME_WB; i++) 
		{
			f_sMDCT_wb_postprocess[i] = MDCT_wb[i] / (16.0f * Sqrt(5.0f));
		}

		f_PCMSWB_TDAC_inv_mdct(fy_low,f_sMDCT_wb_postprocess,dec_st->fPrev_wb,0,dec_st->fCurSave_wb);

		IF_Coef_Generator( MDCT_wb, mode, f_scoef_SWB, f_sfEnv, f_sfGain, 
			                     dec_st->pre_fEnv, dec_st->pre_mode, noise_flag, bit_switch_flag, dec_st->modeCount, &dec_st->Seed);

		if (mode == TRANSIENT)
		{
			for (i=0; i<SWB_TENV; i++)
			{
				j = GetBit(pBit, 4);
				f_sTenv_SWB[i] = (Float) (1 << j);
			}
			T_modify_flag = (Short) GetBit(pBit, 1);
		}

		/* copy the BWE parameters and decoded coefficients */
		*sig_Mode = mode;
		movF(SWB_NORMAL_FENV, f_sfEnv, dec_st->pre_fEnv);
		dec_st->prev_enerL = f_senerL;
		*index_g = index_fGain;				

		return (T_modify_flag);
	} /*end of ploss_status!=1*/
}

void T_Env_Postprocess(Float *fOut, Float *tPre)
{
	int i;  

	fOut[0] = 0.85f*fOut[0] + 0.08f*tPre[HALF_SUB_SWB_T_WIDTH-1]
	        + 0.05f*tPre[HALF_SUB_SWB_T_WIDTH-2] + 0.02f*tPre[HALF_SUB_SWB_T_WIDTH-3];

	fOut[1] = 0.85f*fOut[1] + 0.08f*fOut[0]
	        + 0.05f*tPre[HALF_SUB_SWB_T_WIDTH-1] + 0.02f*tPre[HALF_SUB_SWB_T_WIDTH-2];

	fOut[2] = 0.85f*fOut[2] + 0.08f*fOut[1]
	        + 0.05f*fOut[0] + 0.02f*tPre[HALF_SUB_SWB_T_WIDTH-1];

	for (i = 3; i < SWB_T_WIDTH; i++)
	{
		fOut[i] = 0.85f*fOut[i] + 0.08f*fOut[i-1]
		        + 0.05f*fOut[i-2] + 0.02f*fOut[i-3];
	}

	return;
}

Short bwe_dec_timepos( int sig_Mode,
					Float *Tenv_SWB,
					Float *coef_SWB,
					Float *y_hi,       /* (o): Output higher-band signal */
					void *work,         /* (i/o): Pointer to work space        */
					int erasure,
					int T_modify_flag
					)
{
	BWE_state_dec *dec_st = (BWE_state_dec *)work;
	Float fOut[SWB_T_WIDTH];
	Float fSpectrum[SWB_T_WIDTH];
	int i, pos;
	Float enn;
	Float *pit;
	
	Float max_env, ener_prev, atteu;
	Float fY[L_FRAME_WB];

	if (erasure == 0)
	{
		for (i = 0; i < SWB_F_WIDTH; i++) 
		{
			fSpectrum[i] = coef_SWB[i] / (16.0f * Sqrt(5.0f));
		}
		zeroF(ZERO_SWB, &fSpectrum[L_FRAME_WB-ZERO_SWB]);
		movF(L_FRAME_WB, fSpectrum, fY);
	}
	f_PCMSWB_TDAC_inv_mdct(fOut,fY,dec_st->fPrev,(Short) erasure,dec_st->fCurSave);

	/* temporal domain post-processing */
	if ((sig_Mode == TRANSIENT) || (dec_st->pre_mode == TRANSIENT))
	{
		pit = fOut;
		for (i = 0; i < SWB_TENV; i++)
		{
			enn = EPS;
			for (pos = 0; pos < SWB_TENV_WIDTH; pos++)
			{
				enn += (*pit) * (*pit);
				pit++;
			}
			enn = (Float)sqrt(SWB_TENV_WIDTH / enn);
			if (sig_Mode != TRANSIENT)
			{
				Tenv_SWB[i] = dec_st->pre_tEnv * ftEnv_weight[i];
			}
			if (enn > 0)
			{
				pit -= SWB_TENV_WIDTH;
				for (pos = 0; pos < SWB_TENV_WIDTH; pos++)
				{
					*pit *= enn * Tenv_SWB[i];
					pit++;
				}
			}
		}

		if ((sig_Mode == TRANSIENT) && (T_modify_flag == 1))
		{
			max_env = Tenv_SWB[0];
			pos = 0;
			for (i = 1; i < SWB_TENV; i++)
			{
				if (Tenv_SWB[i] > max_env)
				{
					max_env = Tenv_SWB[i];
					pos = i;
				}
			}

			if (pos != 0)
			{
				pit = &fOut[SUB_SWB_T_WIDTH * pos];
				atteu = Tenv_SWB[pos - 1] / Tenv_SWB[pos];
				for (i = 0; i < HALF_SUB_SWB_T_WIDTH; i++)
				{
					*pit *= atteu;
					pit++;
				}
			}
			else
			{
				pit = dec_st->tPre;
				ener_prev = EPS;
				for(i = 0; i < HALF_SUB_SWB_T_WIDTH; i++)
				{
					ener_prev += (*pit) * (*pit);
					pit++;
				}

				ener_prev = Sqrt(ener_prev / HALF_SUB_SWB_T_WIDTH);
				pit = fOut;
				atteu = ener_prev / Tenv_SWB[pos];
				for(i = 0; i < HALF_SUB_SWB_T_WIDTH; i++)
				{
					*pit *= atteu;
					pit++;
				}
			}
		}
	}
	else
	{
		T_Env_Postprocess(fOut, dec_st->tPre);
	}

	for(i=0; i<SWB_T_WIDTH; i+=2)
	{
		fOut[i] *= (-1.0f);
	}

	/* copy decoded sound data to output buffer */
	movF (SWB_T_WIDTH, fOut, y_hi);
	movF( HALF_SUB_SWB_T_WIDTH, &fOut[SWB_T_WIDTH-HALF_SUB_SWB_T_WIDTH], dec_st->tPre );

	if (sig_Mode == TRANSIENT)
	{
		dec_st->pre_tEnv  = Tenv_SWB[3];
	}

	dec_st->pre_mode = sig_Mode;

	return DECODER_OK;
}
