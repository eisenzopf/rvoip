/* ITU G.722 3rd Edition (2012-09) */

/*--------------------------------------------------------------------------
 ITU-T Annex B (ex G.722-SWB) Source Code
 Software Release 1.00 (2010-09)
 (C) 2010 France Telecom, Huawei Technologies, NTT, VoiceAge Corp.
--------------------------------------------------------------------------*/

/*
 *------------------------------------------------------------------------
 *  File: lowband_enc.c
 *  Function: Lower-band encoder
 *------------------------------------------------------------------------
 */
#ifndef HSB_ENH_H
#define HSB_ENH_H

#include "funcg722.h"

Word16 hsbdec_enh(Word16 ih, Word16 ih_enh, Word16 mode_enh,  g722_state *s,
                  Word16 i, UWord16 **pBit_wbenh, Word16 wbenh_flag, Word16 *enh_no, Word32 *i_sum);


void hsbcod_buf_ns(const Word16 sigin[], Word16 code0[], Word16 code1[], g722_state *g722_encoder, void *ptr, Word16 mode,
                   Word16 wbenh_flag, UWord16 **pBit_wbenh);

/**************
 *     tables *
 **************/
extern const Word16   oq4new[16];
extern const Word16   oq3new[8];
extern const Word16   oq3new8[8];
extern const Word16   tresh_enh[4];
extern const Word16   oq4_3new[24]; 

#endif
