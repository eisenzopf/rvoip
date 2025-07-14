/* ITU G.722 3rd Edition (2012-09) */

/*--------------------------------------------------------------------------
 ITU-T Annex B (ex G.722-SWB) Source Code
 Software Release 1.00 (2010-09)
 (C) 2010 France Telecom, Huawei Technologies, NTT, VoiceAge Corp.
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
  Word16 hb_buf[HB_FEC_BUF_LEN]; /* HB signal buffer for FERC */
  Word16 lb_t0;                  /* pitch delay of lowerband  */
  Word16 first_loss_frame;
  Word16 hb_t0;                  /* pitch delay of higherband */
  Word16 att_weight;
  Word16 high_cor;
  Word16 pre_bfi;
} HBFEC_State;

/* Function prototypes */

void   update_hb_buf(Word16 *hb_buf, Word16 *output_hi);
void   copy_lb_pitch(void * SubDecoderL, void * SubDecoderH);
Word16 cor_hb_fec(Word16 * hb_buf, Word16 *hb_pitch_best);

#endif
