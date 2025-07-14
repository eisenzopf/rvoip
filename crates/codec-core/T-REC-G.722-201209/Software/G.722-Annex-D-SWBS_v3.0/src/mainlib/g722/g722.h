/* ITU G.722 3rd Edition (2012-09) */

/*--------------------------------------------------------------------------
 ITU-T Annex B (ex G.722-SWB) Source Code
 Software Release 1.00 (2010-09)
 (C) 2010 France Telecom, Huawei Technologies, NTT, VoiceAge Corp.
--------------------------------------------------------------------------*/

#ifndef G722_H
#define G722_H 200

#include "stl.h"

void *g722_encode_const();
void g722_encode_dest(void *ptr);
void g722_encode_reset(void *ptr);
void g722_encode(Word16 mode, Word16 local_mode, const Word16 *sig, unsigned char *code,
                 unsigned char *code_enh, Word16 mode_enh,
                 void *ptr, Word16 wbenh_flag, UWord16 **pBit_wbenh);


void *g722_decode_const();
void g722_decode_dest(void *ptr);
void g722_decode_reset(void *ptr);
void g722_decode(Word16 mode, const unsigned char *code,
                 const unsigned char *code_enh, Word16 mode_enh,
                 int loss_flag, Word16 *outcode, void *ptr,
                 UWord16 **pBit_wbenh, Word16 wbenh_flag);

#endif /* G722_H */
