/* ITU G.722 3rd Edition (2012-09) */

/*--------------------------------------------------------------------------
 ITU-T G.722 Annex C (ex G.722-SWB-Float) Source Code
 Software Release 1.01 (2012-07)
 (C) 2012 France Telecom, Huawei Technologies, VoiceAge Corp., NTT.
--------------------------------------------------------------------------*/

/*
 *------------------------------------------------------------------------
 *  File: fec_highband.h
 *  Function: Header of high-band frame erasure concealment (FERC)
 *------------------------------------------------------------------------
 */

#ifndef FEC_HIGHBAND_H
#define FEC_HIGHBAND_H


/* Constants for higher-band FERC */

#define MAXPIT                144 /* maximal pitch lag (20ms @8kHz) => 50 Hz */
#define HB_PITCH_SEARCH_RANGE 3
#define ATT_WEIGHT_STEP       164
#define HB_FEC_BUF_LEN        (L_FRAME_NB*3 + 144)	

typedef struct {
  Short hb_buf[HB_FEC_BUF_LEN]; /* HB signal buffer for FERC */
  Short lb_t0;                  /* pitch delay of lowerband  */
  Short first_loss_frame;
  Short hb_t0;                  /* pitch delay of higherband */
  Short att_weight;
  Short high_cor;
  Short pre_bfi;
} HBFEC_State;

/* Function prototypes */

void   update_hb_buf(Short *hb_buf, Short *output_hi);
void   copy_lb_pitch(void * SubDecoderL, void * SubDecoderH);
Short cor_hb_fec(Short * hb_buf, Short *hb_pitch_best);

#endif
