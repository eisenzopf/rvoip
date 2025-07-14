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

#define ENCODER_OK  0
#define ENCODER_NG  1

/* temporal envelop calculation */
Short Icalc_tEnv(  
					   Float *sy,         /* (i/o)   current SWB high band signal  */
		               Float *srms,       /* (o)     log2 of the temporal envelope  */
		               Short *transient,
		                 int preMode,
		                void *work    
		   )

{
	int  i, j,pos = 0;
	int T_modify_flag = 0;
	Float log_rms_fix[NUM_FRAME * SWB_TENV];	
	Float temp_fix, i_max_fix, max_deviation_fix;
	Float max_rise_fix;
	Float  ener_total_fix;
	Float temp32;
	Float  gain_fix;
	Float *pit_fix;
	Float avrg_fix = 0.01f;
	Float avrg1_fix = 0.01f;
	Float ener_front_fix;
	Float ener_behind_fix;
	Float max_rms_fix;
	Float enerEnv = 0;
	BWE_state_enc* enc_st = (BWE_state_enc*)work;

	ener_total_fix = EPS;
	pit_fix = sy;
	movF(NUM_PRE_SWB_TENV, enc_st->log_rms_fix_pre, log_rms_fix);
	ener_total_fix = enc_st->enerEnvPre[0]+ enc_st->enerEnvPre[1];

	for(i = 0; i < SWB_TENV; i++)  /* 0 --- 4 */
	{	
		temp32 =EPS;
		for ( j = 0; j < SWB_TENV_WIDTH; j++ )
		{
			temp32 += (*pit_fix)*(*pit_fix);
			pit_fix++;
		}
		enerEnv +=temp32;
		log_rms_fix[NUM_PRE_SWB_TENV + i] = 0.5f * (Float) Log10( temp32 / SWB_TENV_WIDTH + EPS ) * FAC_LOG2;
	}

	ener_total_fix +=enerEnv; 
	enc_st->enerEnvPre[0] = enc_st->enerEnvPre[1];  
	enc_st->enerEnvPre[1] = enerEnv; 

	gain_fix = 0.5f * (Float) Log10( ener_total_fix / (NUM_FRAME * SWB_T_WIDTH) + EPS) * FAC_LOG2;

	i_max_fix = 0; 
	max_deviation_fix = 0; 
	max_rise_fix = 0; 
	for (i = 0; i < (NUM_FRAME*SWB_TENV); i++)
	{
		if (log_rms_fix[i] > i_max_fix)
		{
			i_max_fix = log_rms_fix[i];
			pos = i;
		}

		temp_fix = (Float) Fabs(log_rms_fix[i]-gain_fix);
		if(temp_fix > max_deviation_fix)
		{
			max_deviation_fix = temp_fix;
		}
	}
	for (i = 0; i < (NUM_FRAME*SWB_TENV-1); i++)
	{
		temp_fix = log_rms_fix[i+1] - log_rms_fix[i];
		if(temp_fix > max_rise_fix) 
		{
			max_rise_fix = temp_fix;
		}
	}

	if ( max_deviation_fix > 3.3f && max_rise_fix > 2.4f && gain_fix > 8.0f )
	{
		*transient = 1;
	}
	else
	{
		*transient = 0;
	}
	if ( *transient == 1 || preMode == TRANSIENT )
	{
		if ( pos >= 4 )
		{
			temp_fix = (Float) (1.0 / pos);
			for (i = 0; i < pos ; i++)
			{
				avrg_fix += log_rms_fix[i] * temp_fix;
			}
			avrg_fix = (log_rms_fix[pos] / avrg_fix);
			if (pos < 8)
			{
				temp_fix = (1.0f / (11 - pos));
				for (i = pos + 1; i < 12; i++)
				{
					avrg1_fix += log_rms_fix[i] * temp_fix;
				}
				avrg1_fix = (log_rms_fix[pos] / avrg1_fix);
			}
		}
		for (i=0; i<SWB_TENV; i++)
		{
			if (i+SWB_TENV == pos && *transient == 1 && avrg_fix > 2.0 && avrg1_fix > 2.0)
			{
				srms[i] = log_rms_fix[i+SWB_TENV] + 0.5f;
			}
			else if(i+SWB_TENV < pos && *transient == 1)
			{
				srms[i] = log_rms_fix[i+SWB_TENV] - 1.0f;
			}
			else
			{
				srms[i] = log_rms_fix[i+SWB_TENV];
			}

			if (srms[i] > 15)
			{
				srms[i] = 15.0f;
			}
			else if (srms[i] < 0)
			{
				srms[i] = 0.0f;
			}
		}

		max_rms_fix = srms[0];
		pos = 0;
		//find the max value and its position
		for (i = 1; i < SWB_TENV; i++)
		{
			if (srms[i] > max_rms_fix)
			{
				max_rms_fix = srms[i];
				pos = i;
			}
		}

		pit_fix = &enc_st->pre_sy[SUB_SWB_T_WIDTH * pos];

		ener_front_fix = EPS;
		for(i = 0; i < HALF_SUB_SWB_T_WIDTH; i++)
		{
			ener_front_fix += (*pit_fix) * (*pit_fix);
			pit_fix++;
		}
		ener_behind_fix = EPS;
		for(i = 0; i < HALF_SUB_SWB_T_WIDTH; i++)
		{
			ener_behind_fix += (*pit_fix) * (*pit_fix);
			pit_fix++;
		}

		if(ener_behind_fix > ener_front_fix)
		{
			T_modify_flag = 1; 
		}
		else
		{
			T_modify_flag = 0; 
		}

	}

	movF(SWB_TENV, &enc_st->log_rms_fix_pre[SWB_TENV], enc_st->log_rms_fix_pre);
	movF(SWB_TENV, &log_rms_fix[NUM_PRE_SWB_TENV], &enc_st->log_rms_fix_pre[SWB_TENV]);
	movF(SWB_T_WIDTH, sy, enc_st->pre_sy);

	return(T_modify_flag);
}

void Cod_fEnv(Float *fEnv, Short *codword, int mode)
{
	int i, j, pos;
	Float minn, dist; 
	Float *pit = fcodebookL; 

	if (mode == 0)
	{
		pit = fcodebookL;
	}
	else if (mode == 1)
	{
		pit = fcodebookH;    
	}

	minn = 100000.0f;
	pos = 0;
	for (i=0; i<VQ_FENV_SIZE; i++)
	{
		dist = 0;
		for (j=0; j<VQ_FENV_DIM; j++)
		{
			dist += (Float) ((fEnv[j] - *pit) * (fEnv[j] - *pit));
			pit++;
		}

		if (dist < minn)
		{
			minn = dist;
			pos = i;
		}
	}

	pit -= (VQ_FENV_SIZE - pos) * VQ_FENV_DIM;
	for (i=0; i<VQ_FENV_DIM; i++)
	{
		fEnv[i] = (Float) pit[i];
	}
	*codword = pos;

	return;
}

/* index fGain */
int cod_fGain(Float *fGain)
{
	int index_fGain;

	index_fGain = (int) (Log10(*fGain) * FAC_LOG2 + 0.5);

	if (index_fGain < 0)
	{
		index_fGain = 0;
	}
	else if (index_fGain > 31)
	{
		index_fGain = 31;
	}

	*fGain = (Float)(1 << index_fGain);

	return(index_fGain);
}

void calc_fEnv(
					 Float fGain,
					 Float *fSpectrum, 
					 int mode, 
					 Float *fEnv, 
					 Short *index_codebook, 
					 Short *index_fEnv, 
					 Float fEnv_unq[]
)
{
	int i,j;
	Float sgain_tmp;
	Float Sphere1, Sphere2;
	Float *spit,*spit1;
	Float en;
	Float fEnv_tmp[SWB_NORMAL_FENV];

	spit = fSpectrum;		
	if (mode == TRANSIENT)
	{
		sgain_tmp = (Float) (TRANSI_FENV_EXPAND / fGain);
		for (i = 0; i < SWB_TRANSI_FENV; i++)
		{
			en = EPS;
			for (j = 0; j < SWB_TRANSI_FENV_WIDTH; j++)
			{
				en += (*spit) * (*spit);
				spit++;
			}
			fEnv[i] = (Float)sqrt( en / SWB_TRANSI_FENV_WIDTH );
			index_fEnv[i] = (int) (fEnv[i] * sgain_tmp + 0.5);

			if (index_fEnv[i] > 15)
			{
				index_fEnv[i] = 15;
			} 
			fEnv_unq[2*i]   = fEnv[i] / fGain;
	  
			fEnv_unq[2*i+1] = fEnv_unq[2*i];
		}
	}
	else
	{
		sgain_tmp = (Float) (1.0 / fGain);
		sgain_tmp = sgain_tmp *sgain_tmp;
		spit = fSpectrum;
		sgain_tmp *= (Float) 0.125;
		for (i = 0; i < SWB_NORMAL_FENV; i++)
		{
			en = EPS;
			for (j = 0; j < FENV_WIDTH; j++)
			{
				en += (*spit) * (*spit);
				spit++;
			}

			fEnv_tmp[i] = (Float) (en * sgain_tmp);
			fEnv[i] = Sqrt( fEnv_tmp[i] );
		}

		/* vector quantize fEnv */
		Sphere1 = (Float) 0.0;
		Sphere2 = (Float) 0.0;
		spit = fEnv_tmp;
		spit1 = &fEnv_tmp[SWB_NORMAL_FENV / 2];
		for (i = 0; i < SWB_NORMAL_FENV / 2; i++)
		{
			Sphere1 += *spit;
			Sphere2 += *spit1;
			spit++;
			spit1++;
		}

		if (Sphere1 > 1.69f)
		{
			index_codebook[0] = 1;
		}

		movF( SWB_NORMAL_FENV, fEnv, fEnv_unq );

		Cod_fEnv( fEnv, index_fEnv, index_codebook[0] );
		if (Sphere2 > 1.69f)
		{
			index_codebook[1] = 1;
		}
		Cod_fEnv( &fEnv[4], &index_fEnv[1], index_codebook[1] );
	}
	return;
}

void clas_sharp(Short preMod, Float *fSpectrum, Float fGain, 
				Short *sharpMod, Short *noise_flag, Float fpreGain
				, BWE_state_enc* enc_st    
				)
{
	int i, j, k, noise;
	Float *input_hi;
	Float sharp[NUM_SHARP];
	Float sharpPeak = 0;
	Float gain_tmp;
	Float peak;
	Float mag;
	Float mean;

	input_hi = fSpectrum;
	k=0;  
	noise = 0; 
	sharpPeak = 0.0;

	for (i = 0; i < NUM_SHARP; i ++)
	{

		peak = 0.0f;
		mean = 0.0f;
		for (j = 0; j < SHARP_WIDTH; j ++)
		{
			mag = (Float) fabs(*input_hi);
			if (mag > peak) 
			{
				peak = mag;
			}
			mean += mag;
			input_hi ++;
		}

		if(mean != 0.0f) 
		{
			sharp[i] = (Float) (peak * 5.0f / (mean - peak));
		}
		else 
		{
			sharp[i] = 0.0f;
		}

		if (sharp[i] > 4 && peak > 10) 
		{
			k += 1;
		}
		else if (sharp[i] < 2.5)
		{
			if (sharp[i] > 0)
			{
				noise += 1;
			}
		}
		if (sharp[i] > sharpPeak)
		{
			sharpPeak = sharp[i];
		}

	}
	if(preMod == HARMONIC)
	{
		j = 4;
	}
	else if (preMod == TRANSIENT)
	{
		j = 7;
	}
	else
	{
		j = 5;
	}

	gain_tmp = fGain * fpreGain;
	if(k >= j && gain_tmp > 0.5f && gain_tmp < 1.8f)
	{
		*sharpMod = 1;
		if (enc_st->modeCount < 8)
		{
			enc_st->modeCount += 1;
		}
	}
	else
	{
		*sharpMod = 0;
		if (enc_st->modeCount > 0)
		{
			enc_st->modeCount -= 1;
		}
	}

	if (enc_st->modeCount >= 2)
	{
		*sharpMod = 1;
	}

	if (noise > 6 && sharpPeak < 3.5f)
	{
		*noise_flag = 1;
	}
	else
	{
		*noise_flag = 0;
	}

	return;
}

void norm_spectrum_bwe( Float* fSpectrum, Float* fGain , int nb_coef)
{
	int i;
	Float en;
	Float *pit;

	en = EPS;
	pit = fSpectrum;
	for (i=0; i<nb_coef; i++) 
	{
		en += (*pit) * (*pit);
		pit++;
	}

	*fGain = (Float) sqrt(en / nb_coef);

	return;
}

void QMF_mirror( Float *s, int l ) 
{
	Short i; 

	for (i = 0; i < l; i += 2)
	{
		s[i] = -s[i];
	}
}

void* bwe_encode_const(void)
{
	BWE_state_enc  *enc_st=NULL;

	enc_st = (BWE_state_enc *)malloc( sizeof(BWE_state_enc) );
	if (enc_st == NULL) return NULL;

	bwe_encode_reset( (void *)enc_st );

	return (void *)enc_st;
}

void  bwe_encode_dest( void *work )
{
	BWE_state_enc  *enc_st=(BWE_state_enc *)work;

	if (enc_st != NULL)
	{
		free( enc_st );
	}
}

Short bwe_encode_reset( void *work )
{
	BWE_state_enc  *enc_st=(BWE_state_enc *)work;
	int  i;

	if (enc_st != NULL)
	{
		enc_st->preMode = 0;
		enc_st->preGain = 0.0;
		enc_st->modeCount = 0;
		for (i = 0; i < SWB_T_WIDTH; i++)
		{
			enc_st->fIn[i] = 0.0f;
		}
		for (i = 0; i < (NUM_FRAME -1 ) * SWB_TENV; i++)
		{
			enc_st->stEnvPre[i] = 0;
		}
		for (i = 0; i < NUM_PRE_SWB_TENV ; i++)
		{
			enc_st->log_rms_fix_pre[i] = 0.0;
		}
		for (i = 0; i < NUM_FRAME ; i++)
		{
			enc_st->enerEnvPre[i] = 0.0;
		}
		for (i = 0; i < SWB_T_WIDTH ; i++)
		{
			enc_st->pre_sy[i] = 0.0;
		}
	}

	return ENCODER_OK;
}

Short bwe_enc(
			   Float          fBufin[],           /* (i): Input super-higher-band signal */
			   unsigned short **pBit,             /* (o): Output bitstream               */
			   void           *work,              /* (i/o): Pointer to work space        */
			   Float          *tEnv,              /* (i) */
			   Short          transi,
			   Short          *cod_Mode,
			   Float          *f_Fenv_SWB,        /* (o) */
			   Float          *fSpectrum,         /* (o) */
			   Short          *index_g,
			   Short          T_modify_flag,
			   Float          fEnv_unq[]          /* (o) */
			   )
{ 
	Short sharpMod, mode, index_fGain;
	Short index_fEnv[SWB_TRANSI_FENV], index_fEnv_codebook[NUM_FENV_CODEBOOK]={0}, index_fEnv_codeword[NUM_FENV_VECT];
	Short noise_flag;
	Float fGain;
	Float fenn;	
	BWE_state_enc  *enc_st=(BWE_state_enc *)work;

	Short i;
	Float fY[80];

	QMF_mirror( fBufin, L_FRAME_WB );

	f_bwe_mdct( enc_st->fIn, fBufin, fY, 1);
	movF( SWB_F_WIDTH , fY , fSpectrum );

	for (i=0; i<SWB_F_WIDTH; i++) 
	{
		fSpectrum[i] = fSpectrum[i] *16.0f * Sqrt(5.0f);
	}
	/* Normalize MDCT coefficients with RMS */
	norm_spectrum_bwe( fSpectrum, &fGain, SWB_F_WIDTH );
	fenn = (Float) (1 / fGain);
	
	if (transi == 1 ||enc_st->preMode == TRANSIENT) 
	{
		mode = TRANSIENT;      
		*cod_Mode = mode; 
		s_PushBit(mode, pBit, 2);

		/* encode fGain with 5bits */
		index_fGain = cod_fGain( &fGain);
		s_PushBit(index_fGain, pBit, 5 );

		calc_fEnv( fGain, fSpectrum, mode, f_Fenv_SWB, index_fEnv_codebook, index_fEnv, fEnv_unq );

		for (i = 0; i < SWB_TRANSI_FENV; i++)
		{			
			s_PushBit(index_fEnv[i], pBit, 4);
		}

		if (transi != 1)
		{
			mode = NORMAL;
		}
		enc_st->modeCount = 0;
		for (i=0; i<SWB_TRANSI_FENV; i++)
		{
			f_Fenv_SWB[i] = index_fEnv[i]*INV_TRANSI_FENV_EXPAND;
			f_Fenv_SWB[i] = f_Fenv_SWB[i]* fGain;
		}

		for (i=0; i<SWB_TENV; i++)
		{
			s_PushBit((Short)(tEnv[i]+0.5f), pBit, 4);
		}

		s_PushBit( (Short) T_modify_flag, pBit, 1 );

	}
	else /* not transient */
	{
		clas_sharp(enc_st->preMode, fSpectrum, fGain,
			             &sharpMod, &noise_flag, enc_st->preGain , enc_st );
		
		/* encode fGain with 5bits */
		index_fGain = cod_fGain( &fGain);

		if ((sharpMod == 1) || (enc_st->preMode == HARMONIC))
		{
			mode = HARMONIC;
			*cod_Mode = mode;
			s_PushBit( mode, pBit, 2 );
			if (sharpMod != 1)
			{
				mode = NORMAL;
			}
		}
		else
		{
			mode = NORMAL;
			*cod_Mode = mode;
			s_PushBit(mode, pBit, 1 );
			s_PushBit(noise_flag, pBit, 1 );
		}

		calc_fEnv( fGain, fSpectrum, mode, f_Fenv_SWB, index_fEnv_codebook, index_fEnv_codeword   
			, fEnv_unq );

		s_PushBit(index_fGain, pBit, 5 );

		s_PushBit(index_fEnv_codebook[0], pBit, 1 );
		s_PushBit(index_fEnv_codebook[1], pBit, 1 );

		s_PushBit(index_fEnv_codeword[0], pBit, 6 );
		s_PushBit(index_fEnv_codeword[1], pBit, 6 );
		for (i=0; i<SWB_NORMAL_FENV; i++)
		{
			f_Fenv_SWB[i] *= fGain;
		}
	}

	enc_st->preMode = mode; 
	enc_st->preGain = fenn;
	*index_g = index_fGain;      

	return ENCODER_OK;
}
