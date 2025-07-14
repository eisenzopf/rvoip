/* ITU G.722 3rd Edition (2012-09) */

/*--------------------------------------------------------------------------
 ITU-T G.722 Annex C (ex G.722-SWB-Float) Source Code
 Software Release 1.01 (2012-07)
 (C) 2012 France Telecom, Huawei Technologies, VoiceAge Corp., NTT.
--------------------------------------------------------------------------*/
#ifndef HSB_ENH_H
#define HSB_ENH_H

#include "funcg722.h"


/**************
 *     tables *
 **************/
Short fl_hsbdec_enh(Short ih, Short ih_enh, Short mode,  g722_state *s,
                  Short i, unsigned short **pBit_wbenh, Short wbenh_flag, Short *enh_no, Float *sum_ma_dh_abs);

Short hsbdec_enh(Short ih, Short ih_enh, Short mode_enh,  g722_state *s,
                  Short i, unsigned short **pBit_wbenh, Short wbenh_flag, Short *enh_no, long *i_sum);


void fl_hsbcod_buf_ns(const Short sigin[], Short code0[], Short code1[], g722_state *g722_encoder, void *ptr, Short mode,
                   Short wbenh_flag, unsigned short **pBit_wbenh);

/**************
 *     tables *
 **************/
extern const Short   oq4new[16];
extern const Short   oq3new[8];
extern const Short   oq3new8[8];
extern const Short   tresh_enh[4];
extern const Short   oq4_3new[24]; 


#endif
