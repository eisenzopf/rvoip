/* ITU G.722 3rd Edition (2012-09) */

/*--------------------------------------------------------------------------
 ITU-T G.722 Annex C (ex G.722-SWB-Float) Source Code
 Software Release 1.01 (2012-07)
 (C) 2012 France Telecom, Huawei Technologies, VoiceAge Corp., NTT.
--------------------------------------------------------------------------*/

#ifndef G722_H
#define G722_H 200

void fl_g722_encode_reset(void *ptr);
void *fl_g722_encode_const();
void fl_g722_encode_dest(void *ptr);
void g722_encode(Short mode, Short local_mode, const Short *sig, unsigned char *code,
                 unsigned char *code_enh, Short mode_enh, /* mode_enh = 1 -> high-band enhancement layer */
                 void *ptr, Short wbenh_flag, unsigned short **pBit_wbenh
                 );

void *g722_decode_const();
void g722_decode_dest(void *ptr);
void g722_decode_reset(void *ptr);

void g722_decode(Short mode, const unsigned char *code,
                 const unsigned char *code_enh, Short mode_enh,
                 int loss_flag, Short *outcode,
                 void *ptr, unsigned short **pBit_wbenh, Short wbenh_flag
                 );


#endif /* G722_H */
