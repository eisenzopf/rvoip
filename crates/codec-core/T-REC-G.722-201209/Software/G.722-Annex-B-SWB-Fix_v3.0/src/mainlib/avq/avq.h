/* ITU G.722 3rd Edition (2012-09) */

/*--------------------------------------------------------------------------
 ITU-T Annex B (ex G.722-SWB) Source Code
 Software Release 1.00 (2010-09)
 (C) 2010 France Telecom, Huawei Technologies, NTT, VoiceAge Corp.
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

#define SWB_F_WIDTH            64

#define ITU_G192_BIT_0 0x007f               /* constants for bitstream packing */
#define ITU_G192_BIT_1 0x0081               /* constants for bitstream packing */

#define INV_CNST_WEAK_FX 32000              /* Q(5) */
#define CNST_WEAK_FX 16777                  /* Q(24) */
#define CNST_WEAK_FX15                      /* Q(19) */
#define QCOEF 9
#define INV_CNST_WEAK_FX2 (INV_CNST_WEAK_FX/2) /* Q(4) */

typedef struct {
  Word16 pre_cod_Mode;
  Word16  sksm;    /* Q(9) */								/* Counted by "pcmswbEncode_const()" */
  Word16  smnl; /* LOW_LEVEL_NUM_MIN */ /* Q(9) */		/* Counted by "pcmswbEncode_const()" */
  Word16 cnt_detzer;		/* Counted by "pcmswbEncode_const()" */
  Word16 detzer_flg;		/* Counted by "pcmswbEncode_const()" */
} AVQ_state_enc;

typedef struct {
  Word16 pre_cod_Mode;                                       
  Word16 scoef_SWB_abs_old[SWB_F_WIDTH];
  Word16 scoef_SWB_abs_oldQ; /* Q(scoef_SWB_abs_oldQ) */ /* Counted by "pcmswbDecode_const()" */
  Word16 prev_zero_vector[N_SV];		/* Counted by "pcmswbDecode_const()" */

  Word16 sbuffAVQ[SWB_F_WIDTH];
  Word16 sprefSp[SWB_F_WIDTH];	/* Counted by "pcmswbDecode_const()" */
  Word16 spreAVQ0[SWB_F_WIDTH];
  Word16 spreAVQ1[SWB_F_WIDTH];	/* Counted by "pcmswbDecode_const()" */
  Word16 detzer_flg;
  Word16 pre_scoef_SWBQ0;
  Word16 pre_scoef_SWBQ1;
} AVQ_state_dec;

/*------------------------------------------------------------------------*
* Prototypes
*------------------------------------------------------------------------*/
void*  avq_encode_const (void);
void   avq_encode_dest (void *work);
Word16 avq_encode_reset (void *work);
Word16 swbl1_encode_AVQ(void *p_AVQ_state_enc, const Word16 scoef_SWB[], const Word16 sEnv_BWE[], Word16  sratio[], const Word16 index_g_5bit, const Word16 cod_Mode, UWord16 *pBst_L1, UWord16 *pBst_L2, const Word16 layers, const Word16 scoef_SWBQ);

void*  avq_decode_const (void);
void   avq_decode_dest (void *work);
Word16 avq_decode_reset (void *work);
void   swbl1_decode_AVQ(void *p_AVQ_state_dec, UWord16 *pBst_L1, UWord16 *pBst_L2, const Word16 *sfEnv_BWE, Word16 *scoef_SWB, const Word16 index_g_5bit, const Word16 cod_Mode, const Word16 layers, Word16 *scoef_SWBQ) ;
void   bwe_avq_buf_reset();

void AVQ_Cod( Word16 *xri, Word16 *xriq, Word16 NB_BITS, Word16 Nsv );
Word16 AVQ_Encmux_Bstr( Word16 xriq[], UWord16 **pBst, const Word16 n_bits, const Word16 Nsv );
Word16 AVQ_Demuxdec_Bstr( UWord16 *pBst, Word16 xriq[], const Word16 nb_bits, const Word16 Nsv );
void Sort( Word16 *ebits, Word16 n, Word16 *idx, Word16 *t );
void loadSubbandEnergy (Word16 cod_Mode, Word16 *sEnv_BWE, Word16 *sFenv_BWE);
#endif
