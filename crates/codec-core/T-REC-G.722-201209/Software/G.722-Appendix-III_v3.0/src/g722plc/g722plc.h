/* ITU G.722 3rd Edition (2012-09) */

/* ITU-T G.722 Appendix III                                                      */
/* Version:       1.0                                                            */
/* Revision Date: Nov.02, 2006                                                   */

/*
  ITU-T G.722 Appendix III ANSI-C Source Code
  Copyright (c)  Broadcom Corporation 2006.
*/

#ifndef G722PLC_H
#define G722PLC_H

#ifndef G722_H
#include "g722.h"
#endif

/* G722 PLC basic parameters */
#define  SF		   16		/* input Sampling Frequency (in kHz) */
#define  LPCO	   8		/* LPC predictor Order */
#define  DECF	   8 		/* DECimation Factor for coarse pitch period search */
#define  DFO      60    /* Decimation Filter Order (for coarse pitch extraction) */
#define  FRSZ     160   /* FRame SiZe (in 16 kHz samples) */
#define  WINSZ	   160   /* lpc analysis WINdow SiZe (in 16 kHz samples) */
#define  PWSZ	   240   /* Pitch analysis Window SiZe (in 16 kHz samples) */
#define  WML      160   /* pitch refinement Waveform-Matching window Length */
#define  MAXPP	   265	/* MAXimum Pitch Period (in 16 kHz samples) */
#define  MINPP    40    /* MINimum Pitch Period (in 16 kHz samples) */

#define  GATTST	2     /* frame index into erasure to start gain atten */
#define  GATTEND	6     /* frame index into erasure to stop gain atten */
#define	OLAL	   20    /* OLA Length for 1st bad frame in erasure */
#define  OLALG	   40    /* OLA Length for 1st Good frame after erase */
#define  SOLAL	   8     /* Short OverLap-Add window Length, first good frame */
#define  PPHL     5     /* Pitch Period History buffer Length (frames) */
#define  MLO      20    /* figure of Merit LOw threshold */
#define	MHI      28    /* figure of Merit HIgh threshold */ 
#define  MAXOS    28    /* MAXimum # of samples in waveform OffSet for time warping */

/* derived parameters and constants */
#define  FRSZD    (FRSZ/DECF)       /* FRame SiZe in Decimated domain */
#define  MAXPPD   34				      /* ceil(MAXPP/DECF); MAX PP in Decimated domain */
#define  MAXPPD1  (MAXPPD+1)        /* MAXPPD + 1 */
#define  MINPPD   5                 /* floor(MINPP/DECF); MIN PP in Decimated domain */
#define  PWSZD    (PWSZ/DECF)	      /* Pitch analysis Window SiZe in Decimated domain */
#define  XQOFF    (WML+MAXPP+1)	   /* xq[] offset before current frame */
#define  LXQ      (XQOFF+FRSZ)	   /* Length of xq[ ] buffer */
#define  LXD      (MAXPPD+1+PWSZD)  /* Length of xwd[ ] (input X[ ] weighted and Decimated) */
#define  XDOFF    (LXD-FRSZD)       /* XwD[ ] array OFFset for current frame */


/* the following are used in coarptch */
#define cpp_Qvalue  3
#define cpp_scale   (1<<cpp_Qvalue)
#define HMAXPPD (MAXPPD/2)
#define M1  (MINPPD-1)
#define M2  MAXPPD1
#define HDECF (DECF/2)
#define TH1       23921    /* first threshold for cor*cor/energy   */
#define TH2       13107    /* second threshold for cor*cor/energy  */
#define LPTH1     25559    /* Last Pitch cor*cor/energy THreshold 1 */
#define LPTH2     14090    /* Last Pitch cor*cor/energy THreshold 2 */
#define MPDTH     1966     /* Multiple Pitch Deviation THreshold */
#define SMDTH     3113     /* Sub-Multiple pitch Deviation THreshold  0.125 */
#define MPTH4     9830
#define MAX_NPEAKS  7      /* MAXimum Number of PEAKS in coarse pitch search */

#define	UPBOUND  16384   /* UPper BOUND of scaling factor for plc periodic extrapolation Q14 */
#define	DWNBOUND -16384  /* LOwer BOUND of scaling factor for plc periodic extrapolation Q14 */

#define AHP             31785 /* 0.97 in Q15 */
#define W_NBH_TRCK		31785 /* 0.97 in Q15 */
#define W_NBH_TRCK_M1	  983 /* 0.03 in Q15 */
#define W_NBH_CHNG		32512 /* 127/128 in Q15 */
#define W_NBH_CHNG_M1	  256 /* 1/128 in Q15 */
#define W_NBL_TRCK		31785 /* 0.97 in Q15 */
#define W_NBL_TRCK_M1	  983 /* 0.03 in Q15 */
#define W_NBL_CHNG		32512 /* 127/128 in Q15 */
#define W_NBL_CHNG_M1	  256 /* 1/128 in Q15 */

struct WB_PLC_State {
Word32   energymax32;
Word32   cormax;
Word16   wsz;
Word16   scaled_flag;
Word16   xq[LXQ+24+MAXOS];
Word16   stsyml[LPCO];
Word16   al[1+LPCO];
Word16   alast[1+LPCO];
Word16   ppt;
Word16   stwpml[LPCO];
Word16   xwd[XDOFF];
Word16   xwd_exp;
Word16   dfm[DFO];
Word16   scaler;
Word16   merit;
Word16   ptfe;
Word16   ppf;
Word16   ppinc;
Word16   pweflag;
Word16   cpplast;
Word16   pph[PPHL];
Word16   pp;
Word16   cfecount;
Word16   ngfae;
Word16   nfle;
Word16   avm;
Word16   lag;

/* State variables to interface with G.722 */

/* For low-band */
Word16   psml_mean;
Word16   nbpl_mean1;
Word16   nbpl_mean2;
Word16   nbpl_trck;
Word16   nbpl_chng;
Word16   pl_postn;
Word16   lb_reset;

/* For high-band */
Word16   nbph_mean;
Word16   nbph_trck;
Word16   nbph_chng;
Word16   nbh_mode;
Word16   hp_flag;
Word16   nbph_lp;
Word16   ph_postn;
Word16   hb_reset;

Word16   rhhp_m1;
Word16   rh_m1;
Word16   phhp_m1;
Word16   ph_m1;

short	   sb_sample;

/* the following copy is needed for rephasing */
Word16   cpl_postn;
Word16   cph_postn;
Word16   crhhp_m1;
Word16   crh_m1;
Word16   cphhp_m1;
Word16   cph_m1;

g722_state ds;

Word16   lb[MAXOS+11];
Word16   hb[MAXOS+11];

};

/*-----------------------------------------------------------------------------
 * Function: G722DecWithPLC()
 *
 * Description: highest level G722 decoder function for use with PLC.  This 
 *              function should be called for received frame AND lost frames.
 *
 * Inputs:  *chan    - pointer to buffer containing the channel indices for
 *                     the current frame.
 *          mode     - G722 decoder mode.
 *          blocksize- number of samples per frame - MUST be a multiple of 160.
 *          *ds      - pointer to the structure containing the current
 *                     decoder state memory.
 *          *plc     - pointer to the structure containing the current
 *                     plc state memory.
 *          bfi      - bad frame indicator (0=good frame, 1=lost frame)
 *
 * Outputs: *output  - output samples(=blocksize) written to memory starting
 *                     from this pointer.
 *---------------------------------------------------------------------------*/
short G722DecWithPLC(short *chan, short *output, short mode, short blocksize,
                 g722_state *ds, struct WB_PLC_State *plc, short bfi);


/*-----------------------------------------------------------------------------
 * Function: Reset_WB_PLC()
 *
 * Description: reset the plc state variables
 *
 * Inputs:  *plc  - pointer to plc state memory
 *
 * Outputs: *plc  - values reset
 *---------------------------------------------------------------------------*/
void Reset_WB_PLC(struct WB_PLC_State *plc);

/*-----------------------------------------------------------------------------
 * Function: WB_PLC()
 *
 * Description: PLC function called in good frames
 *
 * Inputs:  *plc  - pointer to plc state memory
 *          *out  - pointer to output buffer
 *          *inbuf- pointer to the good frame input
 *
 * Outputs: *out  - potentially modified output buffer
 *---------------------------------------------------------------------------*/
void	WB_PLC(struct 	WB_PLC_State *plc, Word16 	*out,Word16 	*inbuf);

/*-----------------------------------------------------------------------------
 * Function: WB_PLC_erasure()
 *
 * Description: PLC function called in bad frames
 *
 * Inputs:  *plc  - pointer to plc state memory
 *          *out  - pointer to output buffer
 * Outputs: *qdb  - extrapolated samples beyond the output buffer required
 *                  for qmf memory, ringing, and rephasing
 *---------------------------------------------------------------------------*/
void WB_PLC_erasure(struct  WB_PLC_State *plc, Word16 *out, Word16 *qdb);

void Autocorr(
Word32  rl[],       /* (o) : Autocorrelations lags  */
Word16  x[],        /* (i) : Input signal       */
Word16  window[],   /* (i) : LPC Analysis window    */
Word16  l_window,   /* (i) : window length      */
Word16  m);     /* (i) : LPC order      */

void Spectral_Smoothing(
Word16 m,         /* (i)     : LPC order                    */
Word32 rl[],      /* (i/o)   : Autocorrelations  lags       */
Word16 lag_h[],   /* (i)     : SST coefficients  (msb)      */
Word16 lag_l[]);  /* (i)     : SST coefficients  (lsb)      */

void Levinson(
Word32 Rl[],      /* (i)       : Rh[M+1] Vector of autocorrelations lags  */
Word16 A[],       /* (o) Q12   : A[M]    LPC coefficients                 */
Word16 old_A[],   /* (i/o) Q12 : old_A[M+1] old LPC coefficients          */
Word16 m);        /* (i)       : LPC order                    */

Word16 coarsepitch(
Word16  *xw,            /* (i) Q1 weighted low-band signal frame */
Word16 cpp);

void decim(
Word16  *xw,
Word16  *xwd,
struct  WB_PLC_State *cstate);

void azfilterQ0_Q1(
Word16 a[],    /* (i) Q12 : prediction coefficients                     */
Word16 m,      /* (i)     : LPC order                                   */
Word16 x[],    /* (i)     : speech (values x[-m..-1] are needed         */
Word16 y[],    /* (o)     : residual signal                             */
Word16 lg);    /* (i)     : size of filtering                           */

void apfilterQ0_Q0(
Word16 a[],     /* (i) Q12 : a[m+1] prediction coefficients   (m=10)  */
Word16 m,   	/* (i)     : LPC order                */
Word16 x[],     /* (i)     : input signal                             */
Word16 y[],     /* (o)     : output signal                            */
Word16 lg,      /* (i)     : size of filtering                        */
Word16 mem[]    /* (i/o)   : memory associated with this filtering.   */
); 

void apfilterQ1_Q0(
Word16 a[],     /* (i) Q12 : a[m+1] prediction coefficients   (m=10)  */
Word16 m,   	/* (i)     : LPC order                */
Word16 x[],     /* (i)     : input signal                             */
Word16 y[],     /* (o)     : output signal                            */
Word16 lg,      /* (i)     : size of filtering                        */
Word16 mem[]    /* (i/o)   : memory associated with this filtering.   */
); 

Word16  merit(
				Word16  *xq,
				Word16  wsz,
				Word32  cormax,
				Word32  energymax32,
				Word16  scaled_flag);

Word16  prfn(
				Word16  *ptfe,      /* (o) Q14 pitch tap */
				Word32  *cormax,
				Word32  *energymax32,
				Word16  *ppt,
				Word16  *wsz,
				Word16  *sflag,
				Word16  *xq,     /* (i) quantized signal from last sub-frame */
				Word16  pplast);     /* (i) pitch period from last subframe */

/*-----------------------------------------------------------------------------
 * Function: dltdec()
 *
 * Description: partial decoding of G722 "dlt" signal.
 *
 * Inputs:  *code  - pointer to channel indices
 *          detl   - detl memory from previous sample
 *          nbl    - nbl  memory from previous sample
 *          Nsamples - number of samples of dlt to generate
 * Outputs: *out  - decoded dlt signal
 *---------------------------------------------------------------------------*/
void dltdec(short *code, short detl, short nbl, short *out, short Nsamples);

/*-----------------------------------------------------------------------------
 * Function: filtdlt()
 *
 * Description: filtering of dlt signal by a fixed pole-zero filter
 *
 * Inputs:  *in      - pointer to in buffer containing dlt signal
 *          *s       - pointer to g722 state
 *          Nsamples - number of samples to filter
 * Outputs: *out     - output signal
 *---------------------------------------------------------------------------*/
void filtdlt(short *in, g722_state *s, short *out, short Nsamples);

/*-----------------------------------------------------------------------------
 * Function: hsbupd()
 *
 * Description: Updates the high-band ADPCM decoder state memory during
 *              lost packets.
 *
 * Inputs:  *plc     - plc state memory
 *          *s       - G.722 state memory
 *          *out     - high-band PLC component
 *          Nsamples - frame length
 *
 * Outputs: *s       - G.722 state memory
 *---------------------------------------------------------------------------*/
void hsbupd(struct WB_PLC_State *plc, g722_state *s, short *out, short Nsamples);

/*-----------------------------------------------------------------------------
 * Function: lsbupd()
 *
 * Description: Updates the low-band ADPCM decoder state memory during
 *              lost packets.
 *
 * Inputs:  *plc     - plc state memory
 *          *s       - G.722 state memory
 *          *out     - low-band PLC component
 *          Nsamples - frame length
 *
 * Outputs: *s       - G.722 state memory
 *---------------------------------------------------------------------------*/
void lsbupd(struct WB_PLC_State *plc, g722_state *s, short *out, short Nsamples);

/*-----------------------------------------------------------------------------
 * Function: plc_adaptive_prediction()
 *
 * Description: Updates ADPCM predictor coefficients and filter memory - sample-
                based.
 *
 * Inputs:  *d  - difference signal sample
 *          *b  - zero-section coefficients
 *          *a  - pole-section coefficients
 *          *p  - partially reconstructed signal sample
 *          safetythres - pole section safety threshold
 *          *r  - reconstructed signal sample
 *
 * Outputs: *b  - zero-section coefficients
 *          *a  - pole-section coefficients
 *          *sz - zero-section predicted signal sample
 *          s   - predicted signal sample
 *---------------------------------------------------------------------------*/
Word16 plc_adaptive_prediction(Word16 *d, Word16 *b, Word16 *a, Word16 *p, 
							   Word16 safetythres, Word16 *r, Word16 *sz);

/*-----------------------------------------------------------------------------
 * Function: plc_lsbdec()
 *
 * Description: PLC low-band decoding - sample-based.
 *
 * Inputs:  ilr  - index
 *          mode - G.722 mode
 *          rs   - reset flag
 *          *s   - G.722 state
 *
 * Outputs: yl   - low-band reconstructed signal sample
 *          *s   - G.722 state
 *---------------------------------------------------------------------------*/
Word16 plc_lsbdec (Word16 ilr, Word16 mode, Word16 rs, g722_state *s, Word16 psml);

/*-----------------------------------------------------------------------------
 * Function: plc_hsbdec()
 *
 * Description: PLC high-band decoding - sample-based.
 *
 * Inputs:  ih   - index
 *          *s   - G.722 state
 *          *plc - plc state
 *          *pNBPHlpfilter - pointer to lp filter function
 *          *pDCremoval - pointer to DC removal function
 *          inv_frames_int - 
 *          inv_frames_frc - 
 *          sample - sample number
 *          rs   - reset flag (not used)
 *
 * Outputs: yh   - high-band reconstructed signal sample
 *          *s   - G.722 state
 *---------------------------------------------------------------------------*/
Word16 plc_hsbdec (Word16 ih,Word16 rs, g722_state *s, struct WB_PLC_State *plc,
                   Word16 (*pNBPHlpfilter)( struct WB_PLC_State *plc, Word16 inv_frames_int, 
                     Word16 inv_frames_frc, Word16 nbph, Word16 sample), 
                     Word16 (*pDCremoval)(Word16 rh0, Word16 *rhhp, Word16 *rhl),
                     Word16 inv_frame_int, Word16 inv_frame_frc, Word16 sample);

/*-----------------------------------------------------------------------------
 * Function: quantl_toupdatescaling_logscl()
 *
 * Description: update low-band adaptive log scale factor - sample-based.
 *
 * Inputs:  el   - error signal sample
 *          detl - current linear scaling factor
 *          nbl  - previous log scaling factor
 *
 * Outputs: nbpl - updated log scaling factor
 *---------------------------------------------------------------------------*/
Word16 quantl_toupdatescaling_logscl (Word16 el, Word16 detl, Word16 nbl);

/*-----------------------------------------------------------------------------
 * Function: reset_lsbdec()
 *
 * Description: Reset of G.722 low-band decoder.
 *
 * Inputs:  *s  - G.722 state memory
 *
 * Outputs: *s  - G.722 state memory
 *---------------------------------------------------------------------------*/
void reset_lsbdec (g722_state *s);

/*-----------------------------------------------------------------------------
 * Function: hsbdec_resetg722()
 *
 * Description: Reset of G.722 high-band decoder.
 *
 * Inputs:  *s  - G.722 state memory
 *
 * Outputs: *s  - G.722 state memory
 *---------------------------------------------------------------------------*/
void hsbdec_resetg722(g722_state *s);

/*-----------------------------------------------------------------------------
 * Function: reset_hsbdec()
 *
 * Description: Reset of G.722 high-band decoder and related PLC variables.
 *
 * Inputs:  *s   - G.722 state memory
 *          *plc - PLC state memory
 *
 * Outputs: *s   - G.722 state memory
 *          *plc - PLC state memory
 *---------------------------------------------------------------------------*/
void reset_hsbdec (g722_state *s, struct WB_PLC_State *plc);

/*-----------------------------------------------------------------------------
 * Function: NBPHlpfilter()
 *
 * Description: LP pass filter on nbph - sample-based.
 *
 * Inputs:  *plc - plc state memory
 *          inv_frames_int - 
 *          inv_frames_frc - 
 *          nbph - high-band log scale factor
 *          sample - sample number
 *
 * Outputs: nbph - high-band log scale factor
 *---------------------------------------------------------------------------*/
Word16 NBPHlpfilter( struct WB_PLC_State *plc, Word16 inv_frames_int, 
                     Word16 inv_frames_frc, Word16 nbph, Word16 sample);

/*-----------------------------------------------------------------------------
 * Function: NBPHnofilter()
 *
 * Description: No LP pass filter on nbph - sample-based.
 *
 * Inputs:  *plc - plc state memory
 *          inv_frames_int - 
 *          inv_frames_frc - 
 *          nbph - high-band log scale factor
 *          sample - sample number
 *
 * Outputs: nbph - high-band log scale factor
 *---------------------------------------------------------------------------*/
Word16 NBPHnofilter( struct WB_PLC_State *plc, Word16 inv_frames_int, 
                     Word16 inv_frames_frc, Word16 nbph, Word16 sample);

/*-----------------------------------------------------------------------------
 * Function: DCremoval()
 *
 * Description: DC removal filter (HP filter) - sample based.
 *
 * Inputs:  x0  - signal sample
 *          xhp - previous high-pass filtered signal sample
 *          x1  - previous signal sample
 *
 * Outputs: xhp - high-pass filtered signal sample
 *          xhp - high-pass filtered signal sample
 *          x1  - signal sample
 *---------------------------------------------------------------------------*/
Word16 DCremoval(Word16 x0, Word16 *xhp, Word16 *x1);

/*-----------------------------------------------------------------------------
 * Function: DCremovalMemUpdate()
 *
 * Description: DC removal filter (HP filter).  Only memory update - sample based.
 *
 * Inputs:  x0  - signal sample
 *          xhp - previous high-pass filtered signal sample
 *          x1  - previous signal sample
 *
 * Outputs: x0  - Non-high-pass filtered signal sample
 *          xhp - high-pass filtered signal sample
 *          x1  - signal sample
 *---------------------------------------------------------------------------*/
Word16 DCremovalMemUpdate(Word16 x0, Word16 *xhp, Word16 *x1);


#endif

