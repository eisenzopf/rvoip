/* ITU G.722 3rd Edition (2012-09) */

/*--------------------------------------------------------------------------
 ITU-T G.722 Annex C (ex G.722-SWB-Float) Source Code
 Software Release 1.01 (2012-07)
 (C) 2012 France Telecom, Huawei Technologies, VoiceAge Corp., NTT.
--------------------------------------------------------------------------*/

#ifndef AVQ_h
#define AVQ_h

/*------------------------------------------------------------------------*
* Defines
*------------------------------------------------------------------------*/
#define WIDTH_BAND          8               /* N_SV * WIDTH_BAND = 64 MDCT coefs. */
#define N_SV                8               /* N_SV * WIDTH_BAND = 64 MDCT coefs. */
#define N_SV_2              4               /* N_SV/2 */
#define N_SV_L1             3               /* max. number of subvectors (subbands) to encode in SWBL1 */
#define N_SV_L2             4               /* max. number of subvectors (subbands) to encode in SWBL2 */
#define N_SV_CORR           5               /* max. number of subvectors in correlation search in zero sub-bands filling */
#define N_BITS_AVQ_L1       36              /* number of AVQ bits available in SWBL1 */
#define N_BITS_AVQ_L2       40              /* number of AVQ bits available in SWBL2 */
#define N_BITS_AVQ_L1_PLS   37

#define N_BITS_AVQ_MB   15                  /* number of AVQ bits available in G711EL0 */
#define N_SV_MB         2                   /* number of gain bits available in G711EL0 */ 
#define N_BITS_G_MB         3               /* number of bits to quantize gain in G711EL0 */
#define N_BITS_FILL_MB      4

/* AVQ related constants */
#define NSV_MAX    4                        /* == max(N_SV_L1, N_SV_L2), number of sub-vector max in QVAE, 4*8=32 */

#define TH_ORD_B            4               /* threshold of ord_b for mode selection */
#define LOW_LEVEL_NUM_MIN   15              /* min number of low level in input MDCT coefs. */
#define LOW_LEVEL_NUM_MAX   20              /* min number of low level in input MDCT coefs. */ 

#define MIN_NUM_LOW_LEVEL1  10							/* min number of low level in input MDCT coefs. */
#define MIN_NUM_LOW_LEVEL2  20              /* min number of low level in input MDCT coefs. */

#define N_BITS_GAIN_SWBL1   3               /* constants for gain quantization */
#define N_BITS_FILL_L1      4
#define N_BITS_FILL_L2      4
#define N_BASE_BANDS        3
#define CORR_RANGE_L1       15              /* == (2^N_BITS_FILL_L1)-1 */
#define CORR_RANGE_L2       15              /* == (2^N_BITS_FILL_L2)-1 */
#define DETZER_MAX          20              /* maximum detzer counter number  */

#define ENCODER_OK	0
#define ENCODER_NG	1
#define DECODER_OK	0
#define DECODER_NG	1

#define SWB_F_WIDTH         64

#define ITU_G192_BIT_0 0x007f               /* constants for bitstream packing */
#define ITU_G192_BIT_1 0x0081               /* constants for bitstream packing */

#define INV_CNST_WEAK_FX 32000
#define CNST_WEAK_FX 16777
#define CNST_WEAK_FX15
#define QCOEF 9
#define INV_CNST_WEAK_FX2 (INV_CNST_WEAK_FX/2)

//Floating elements
#define CNST_WEAK_FX_F     0.001f

#define INV_CNST_WEAK_FX_F 1000.0f
#define INV_CNST_WEAK_FX2_F 1000.0f


#define NB_COEF_711_EL0     16              /* constants for spectral envelope coding in PCMSWB coder */

typedef struct {
  Short   s_pre_cod_Mode;
  Float   fksm;  /* Counted by "pcmswbEncode_const()" */
  Short   s_mnl; /* LOW_LEVEL_NUM_MIN */ /* Counted by "pcmswbEncode_const()" */
  Short   s_cnt_detzer;		/* Counted by "pcmswbEncode_const()" */
  Short   s_detzer_flg;		/* Counted by "pcmswbEncode_const()" */
} AVQ_state_enc;

typedef struct {
  Short pre_cod_Mode;                                       
  Short prev_zero_vector[N_SV];		/* Counted by "pcmswbDecode_const()" */
  Short detzer_flg;
  Short pre_scoef_SWBQ0;
  Short pre_scoef_SWBQ1;
  Float fcoef_SWB_abs_old[SWB_F_WIDTH];
  Float fbuffAVQ[SWB_F_WIDTH];
  Float fprefSp[SWB_F_WIDTH];	/* Counted by "pcmswbDecode_const()" */
  Float fpreAVQ0[SWB_F_WIDTH];
  Float fpreAVQ1[SWB_F_WIDTH];	/* Counted by "pcmswbDecode_const()" */
} AVQ_state_dec;

/*------------------------------------------------------------------------*
* Prototypes
*------------------------------------------------------------------------*/
void*  avq_encode_const(void);
void   avq_encode_dest(void *work);
Short avq_encode_reset(void *work);
int swbl1_encode_AVQ(void *p_AVQ_state_enc, const Float scoef_SWB[], const Float sEnv_BWE[], Float  sratio[], const Short index_g_5bit, const Short cod_Mode, unsigned short *pBst_L1, unsigned short *pBst_L2, const Short layers);


void*  avq_decode_const(void);
void   avq_decode_dest(void *work);
Short  avq_decode_reset(void *work);
void   swbl1_decode_AVQ(void *p_AVQ_state_dec, unsigned short *pBst_L1, unsigned short *pBst_L2, const Float *fEnv_BWE, Float *fcoef_SWB, const Short index_g_5bit, const Short cod_Mode, const Short layers);
void   bwe_avq_buf_reset();

void   g711el0_encode_AVQ_flt(const Float *mdct_err, unsigned short *bstr_buff, Float *mdct_err_loc, const Float fEnv_BWE0);
void   g711el0_decode_AVQ_flt (unsigned short *bstr_EL0, Float mdct_err[], const unsigned short *bstr_BWE);

void AVQ_cod( Float *xri, Short *xriq, Short NB_BITS, Short Nsv );
Short AVQ_encmux_bstr( Short xriq[], unsigned short **pBst, const Short n_bits, const Short Nsv );
Short AVQ_demuxdec_bstr( unsigned short *pBst, Short xriq[], const Short nb_bits, const Short Nsv );

void sort( Short *ebits, Short n, Short *idx, Short *t );

void f_loadSubbandEnergy(Short cod_Mode, Float *fEnv_BWE, Float *fFenv_BWE ,Short index_g_5bit);
void f_Sort(Float *ebits, Short n, Short *idx, Float *t);
#endif
