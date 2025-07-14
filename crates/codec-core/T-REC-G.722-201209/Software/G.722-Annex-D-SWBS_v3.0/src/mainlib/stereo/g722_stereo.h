/* ITU G.722 3rd Edition (2012-09) */

/*-----------------------------------------------------------------------------------
 ITU-T G.722-SWBS / G.722 Annex D - Reference C code for fixed-point implementation          
 Version 1.0
 Copyright (c) 2012,
 Huawei Technologies
-----------------------------------------------------------------------------------*/

/*
 *------------------------------------------------------------------------
 *  File: g722_stereo.h
 *  Function: Header of stereo function
 *------------------------------------------------------------------------
 */

#ifndef G722_STEREO_H
#define G722_STEREO_H

#ifdef LAYER_STEREO
#include "stl.h"
#include "pcmswb_common.h"
#include "pcmswb.h"

#define WMOPS_ALL
//#define WMOPS_IDX

#define INIT_STEREO_MONO          100
#define INIT_STEREO_MONO_FEC      10

#define IPD_SYN_START             2
#define IPD_SYN_WB_END_WB         IPD_SYN_START + 7
#define IPD_SYN_WB_END_SWB        IPD_SYN_START + 6
#define IPD_SYN_SWB_END_SWB       IPD_SYN_WB_END_SWB + 16

#define IC_BEGIN_BAND             0
#define IC_END_BAND               20

#define inv_LOG2_10               24660     /* 10/log2(10) (Q13) */
#define LOG2_10                   5443      /* log2(10)/20 (Q15) */
#define LOG2_20                   10885     /* log2(10)/10 (Q15) */
#define inv_FNUM                  4681      /* 1/FNUM (Q15) */
#define NFFT                      160       /* window length for FFT */
#define NB_SB                     20        /* number of subbands in Bark scale */
#define SWB_BN                    2         /* number of sunbans for super higher band */
#define START_ILD                 0           
/*----Fixed constant----------------*/
#define PID2_FQ12                 6434
#define NGPID2_FQ12               -6434
#define PI_FQ12                   12868
#define NGPI_FQ12                 -12868    
#define PI3D2_FQ12                19302
#define PI2_FQ12                  25736
#define NGPI2_FQ12                -25736

#define C04_FQ12                  1638
#define C06_FQ12                  2458
#define C20_FQ12                  8192
#define PID4_FQ12                 3217

#define C002_FQ15                 655
#define C098_FQ15                 32113

#define C098_FQ15_N               -32113

#define C08_FQ15                  26214
#define CPIDNFFT_FQ15             643
#define CPIDNFFT_FQ15_2           161
#define C0025_FQ15                819
#define C03_FQ15                  9830
#define C06_FQ15                  19661

#define M1                        32767
#define M2                        -21
#define M3                        -11943
#define M4                        4936
#define K1                        8192
#define K2                        -4096
#define K3                        340
#define K4                        -10
#define FNUM                      7
#define G722_WB_DMX_DELAY         58

#define G722_SWB_DMX_DELAY        80
#define G722_SWBST_D_COMPENSATE   22

#define C_fx51                    -20480    /* [cos(2*pi/5)+cos(2*2*pi/5)]/2-1 (Q14) */
#define C_fx52                    18318     /* [cos(2*pi/5)-cos(2*2*pi/5)]/2 (Q15) */
#define C_fx53                    -31164    /* -sin(2*pi/5) (Q15) */
#define C_fx54                    -25212    /* -[sin(2*pi/5)+sin(2*2*pi/5)] (Q14) */
#define C_fx55                    11904     /* [sin(2*pi/5)-sin(2*2*pi/5)] (Q15) */
#define C_fx81                    23170     /* 1/sqrt(2) (Q15) */
#define C_fx162                   12540     /* cos(pi*6/16) (Q15) */
#define C_fx163                   21407     /* cos(pi*2/16)*sqrt(2) (Q14) */
#define C_fx164                   17734     /* cos(pi*6/16)*sqrt(2) (Q15) */
#define C_fx165                   30274     /* cos(pi*2/16) (Q15) */
#define G_FX                      26214     /* 1/1.25 (Q15) */
#define NG_FX                     -26214

#define NB_SEG5                   5
#define NB_SEG234                 3
#define HALFSTEPQ3                1024

#define STARTBANDITD              3
#define BANDITD                   10
#define PI_D8_4096                1608
#define PI_2_4096                 25736
#define PI_4096_D_4               3217
#define PI_4096                   12868
#define PI_08_4096                10294
#define PI_1D5_4096               19302
#define STARTBANDPHA              1
#define BANDPHA                   6
#define PHA                       10
#define IPD_FNUM                  10

extern const Word16 nbCoefBand[20];
extern const Word16 maxQ[20]; 
extern const Word16 nbShQ[20]; 

extern const Word16 NFFT_D_2_PI_I[10]; 
extern const Word16 MAX_PHASE_I[10];
extern const Word16 INV_nb_idx[13];

extern const Word16 bands[NB_SB+1];      /* sub-bands in Bark scale */
extern const Word16 tab_ild_q5[31];      /* table for ILD quantization with 5 bits */
extern const Word16 tab_ild_q4[15];      /* table for ILD quantization with 4 bits */
extern const Word16 tab_ild_q3[7];       /* table for ILD quantization with 3 bits */
extern const Word16 tab_ild_q2[4];       /* table for ILD quantization with 2 bits */
extern const Word16 band_region[20];
extern const Word16 band_region_ref[20];

extern const Word16 startBand[4*4];
extern const Word16 band_num[4][4];
extern const Word16 band_index[4][5];

extern const Word16 inv_swb_int_bands[3];
extern const Word16 swb_bands[SWB_BN+1];
extern const Word16 swb_int_bands[4];
extern const Word16 tab_phase_q5[32];    /* table for phase quantization with 5 bits */
extern const Word16 tab_phase_q4[16];    /* table for phase quantization with 4 bits */
extern const Word16 win_D[58]; 
extern const Word16 shift[20];

extern const Word16 c_idx[81];

extern const Word16 phaseCases[8];

extern Word16 c_table10[161];
extern Word16 c_table20[161];
extern Word16 tEnv_weight_s[4];

extern const Word16 indPWQU5[NB_SEG5+1]; 
extern const Word16 bSeg5[NB_SEG5]; 
extern const Word16 ind0Seg5[NB_SEG5]; 
extern const Word16 invStepQ5[NB_SEG5]; 
extern const Word16 halfStepQ5[NB_SEG5]; 
extern const Word16 threshPWQU5[NB_SEG5-1];
extern const Word16 indPWQU4[NB_SEG234+1]; 
extern const Word16 bSeg4[NB_SEG234]; 
extern const Word16 ind0Seg4[NB_SEG234]; 
extern const Word16 *invStepQ4 ; 
extern const Word16 *halfStepQ4; 
extern const Word16 *threshPWQU4;
extern const Word16 indPWQU3[2];
extern const Word16 threshPWQU3[NB_SEG234-1]; 
extern const Word16 initIdxQ3[NB_SEG234]; 
extern const Word16 threshPWQU2[NB_SEG234];
extern const Word16 paramQuantPhase[10];
extern const Word16 itdSTEP[16];

extern const Word32 c_table20_1_d_c1[221];
extern const Word32 c_table20_1_d_c2[221]; 
extern const Word16 c_table10_1[221];
extern const Word16 c_d_table_factor[221];
extern const Word32 c_d_table_factor_d_c1[221];
extern const Word16 ic_table[4];

extern const Word16 band_stat2[4*16]; 
extern const Word16 nbBand_stat[4*4]; 

extern const Word16 threshPhaseMeanStdSMct[3];

extern const unsigned char initial_switch[40];
extern const unsigned char initial_switch2[15];
extern const unsigned char initial_switch3[10];

typedef struct{
    /* left channel */
    Word16 mem_input_left[58];        
    Word32 mem_L_ener[20];     /* energy per subband */

    /* right channel */
    Word16 mem_input_right[58];       
    Word32 mem_R_ener[20];     /* energy per subband */

    /* mono signal */
    Word16 mem_mono_ifft_s[58]; 
    Word16 pre_q_left_en_band[20];
    Word16 pre_q_right_en_band[20];

    Word16 mem_ild_q[20];
    Word16 fb_ITD;
    Word16 fb_IPD;

    Word16 mem_mono[L_FRAME_WB];
    Word16 mem_side[L_FRAME_WB];

    Word16 mem_left[G722_SWB_DMX_DELAY];
    Word16 mem_right[G722_SWB_DMX_DELAY];

    Word16 SWB_ILD_mode;

    Word16 frame_flag_wb;
    Word16 frame_flag_swb;
    Word16 c_flag;
    Word16 pre_flag;
    Word16 num;
    Word16 mem_q_channel_en,mem_q_mono_en;
    Word16 swb_frame_idx;
    Word32 mem_l_enr[SWB_BN],mem_r_enr[SWB_BN],mem_m_enr[SWB_BN];

    //--------------ITD realted-------------------
    Word32 Crxyt[STARTBANDITD+BANDITD];
    Word32 Cixyt[STARTBANDITD+BANDITD];

    Word16 pre_ild_sum_swb[FNUM];
    Word16 pre_ild_sum[FNUM],pre_ild_sum_H[FNUM];
    Word16 pos;

    Word16 idx[IPD_SYN_SWB_END_SWB + 1];

    Word16 ic_idx;
    Word16 ic_flag;
    Word16 ipd_num,ipd_reg_num;
    Word16 pre_Itd;
    Word16 pre_Ipd;

    Word16 std_itd_pos_sm;
    Word16 std_itd_neg_sm;
    Word16 nb_idx_pos_sm;
    Word16 nb_idx_neg_sm;
    Word16 pre_itd_neg;
    Word16 pre_itd_pos;

    Word32 Crxyt2[STARTBANDITD+BANDITD];
    Word32 Cixyt2[STARTBANDITD+BANDITD];

    Word16 phase_mean_buf[10];
    Word16 phase_mean_buf1[10];

    Word16 phase_num;
    Word16 pos1;
    Word16 f_num;
    Word32 energy_bin_sm[STARTBANDITD + BANDITD];

    Word16 en_ratio_sm;
    Word16 phase_mean_std_sm_ct;

    Word32 mem_energyL;
    Word32 mem_energyR;

    Word16 pre_ipd_mean;
    Word16 ipd_reg_num_sm;

    Word16 phase_mean_std_sm;
    Word16 ipd_mean_sm;
}g722_stereo_encode_WORK;

typedef struct {
    Word16 mem_output_L[58];                  
    Word16 mem_output_R[58];                  
    Word16 mem_mono_win[58];           

    Word16 mem_ILD_q[20];        /* ILD quantized per sub-band */

    Word16 c1_swb[SWB_BN];
    Word16 c2_swb[SWB_BN];

    Word16 frame_idx;
    Word16 swb_ILD_mode;
    Word16 pre_swb_ILD_mode;
    Word16 swb_frame_idx;
    Word16 delay;

    Word16 c_flag;
    Word16 pre_flag;
    Word16 pre_ipd_q[IPD_SYN_SWB_END_SWB + 1];

    Word16 pre_fb_itd;
    Word16 fb_ipd;
    Word16 fb_itd;
    Word16 pre_fb_ipd;
    Word16 pre_ild_q[20];
    Word16 mem_mono[L_FRAME_WB];
    Word16 mem_left_mdct[L_FRAME_WB],mem_right_mdct[L_FRAME_WB];
    Word16 pre_norm_left,pre_norm_right;
    Word16 sCurSave_left[L_FRAME_WB],sCurSave_right[L_FRAME_WB];
    Word16 mem_left[G722_SWBST_D_COMPENSATE];
    Word16 mem_right[G722_SWBST_D_COMPENSATE];
    Word16 ic_sm;

    Word16 pre_ic_flag;
    Word16 pre_ic_idx;

    Word16 mem_decorr_real[(NFFT/2 + 1) * 4], mem_decorr_imag[(NFFT/2 + 1) * 4];
    Word16 q_mem[4];

    Word16 log_rms_pre[8];
    Word32 enerEnvPre[2];
    Word16 mode;
    Word16 pre_mode;
    Word16 spGain_sm_wb[36];
    Word16 mdct_mem[L_FRAME_WB];
} g722_stereo_decode_WORK;

Word16 Exp16Array_stereo(Word16 n, Word16 *s_real, Word16 *s_imag);
void get_interchannel_difference(g722_stereo_encode_WORK *w,
                                 Word16* L_real,
                                 Word16* L_imag,
                                 Word16  q_left,
                                 Word16* R_real,
                                 Word16* R_imag,
                                 Word16  q_right,
                                 Word16* ic_idx,
                                 Word16* ic_flag
                                 );

Word16 ild_calculation(Word32 L_ener,Word32 R_ener,Word16 q_left_en,Word16 q_right_en);
Word16 ild_calc_dect(Word32 L_ener,Word32 R_ener,Word16 q_diff_en);

void Phase_syn_ITD(Word16 ipd_diff_q,
                   Word16 mono_dec_real,
                   Word16 mono_dec_imag,
                   Word16 c,
                   Word16 L_mag,
                   Word16 R_mag,
                   Word16 *L_real_syn,
                   Word16 *L_imag_syn,
                   Word16 *R_real_syn,
                   Word16 *R_imag_syn
                   );
void Phase_syn_IPD(Word16 ipd_diff_q,
                   Word16 IPD_q,
                   Word16 mono_dec_real,
                   Word16 mono_dec_imag,
                   Word16 c,
                   Word16 L_mag,
                   Word16 R_mag,
                   Word16 *L_real_syn,
                   Word16 *L_imag_syn,
                   Word16 *R_real_syn,
                   Word16 *R_imag_syn
                   );
void  g722_stereo_encode_reset( void* ptr );
void* g722_stereo_encode_const();
void  g722_stereo_encode_dest( void* ptr );
void downmix(Word16* input_left,   /* i: input L channel */
             Word16* input_right,  /* i: input R channel */
             Word16* mono,         /* o: output mono downmix */
             void*   ptr,
             Word16* bpt_stereo,
             Word16  mode,
             Word16* frame_idx,
             Word16  Ops
             );
void g722_stereo_encode(Word16* L_real,
                        Word16* L_imag,
                        Word16* R_real,
                        Word16* R_imag,
                        Word16  q_left,
                        Word16  q_right,
                        Word16* bpt_stereo,
                        void*   ptr, 
                        Word16* frame_idx,
                        Word16  mode,
                        Word16  Ops
                        );

Word16 ild_attack_detect(Word32* L_ener, /* i: energy per sub-band of L channel */
                         Word32* R_ener, /* i: energy per sub-band of R channel */
                         void*   ptr,
                         Word16* nbShL,
                         Word16* nbShR
                         );

Word16 ild_attack_detect_shb(Word32* L_ener, /* i: energy per sub-band of L channel */
                             Word32* R_ener, /* i: energy per sub-band of R channel */
                             Word16  q_left_en,
                             Word16  q_right_en,
                             void*   ptr
                             );
void *g722_stereo_decode_const();
void g722_stereo_decode_dest(void* ptr );
void g722_stereo_decode_reset(void* ptr );
void g722_stereo_decode(Word16* bpt_stereo,
                        Word16* mono_dec,
                        Word16* left_syn,
                        Word16* right_syn,
                        void*   ptr,
                        Word16  mode,
                        Word16  SWB_WB_flag,
                        Word16  ploss_status,
                        Word16* spGain_sm,
                        Word16  cod_Mode,
                        Word16  stereo_mono_flag
                        );

void stereo_synthesis(Word16* ILD_q,      /* i: ILD quantized */
                      Word16* mono_real,  /* i: mono signal */
                      Word16* mono_imag,
                      Word16  q_mono,
                      Word16* L_real_syn, /* o: L signal synthesis */
                      Word16* L_imag_syn, 
                      Word16* R_real_syn, /* o: R signal synthesis */
                      Word16* R_imag_syn,  
                      Word16* q_left,
                      Word16* q_right,
                      Word16* L_mag,
                      Word16* R_mag,
                      Word16  ploss_status
                      );

void downmix_swb(Word16* input_left, 
                 Word16* input_right, 
                 Word16* mono_swb,
                 Word16* side_swb,
                 void*   ptr
                 );

void G722_stereo_encoder_shb(Word16* mono_in,
                             Word16* side_in,
                             void*   prt,
                             short*  bpt_stereo_swb,
                             Word16  mode,
                             Word16* gain
                             );

void g722_stereo_decoder_shb(Word16* bpt_stereo_swb, 
                             Word16* coef_SWB, 
                             Word16  coef_q,
                             Word16* syn_left_swb_s,
                             Word16* syn_right_swb_s,
                             void*   ptr,
                             Word16  ploss_status,
                             Word16  Mode
                             );

Word16 stereo_dec_timepos(Word16  sig_Mode,
                          Word16* sTenv_SWB,
                          Word16* sIn_Out,      /* (i/o): time domain signal */
                          void*   work,         /* (i/o): Pointer to work space */
                          Word16  T_modify_flag,
                          Word16  channel_flag,
                          Word16  delay,
                          Word16  ratio_s
                          );

Word16 arctan2_fix32( Word32 y, Word32 x );
Word16 spx_cos(Word16 x);
Word16 Round_Phase(Word16 x);

#endif /* LAYER_STEREO */
#endif /* G722_STEREO_H */
