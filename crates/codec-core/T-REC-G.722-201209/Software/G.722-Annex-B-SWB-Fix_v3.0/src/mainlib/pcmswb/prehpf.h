/* ITU G.722 3rd Edition (2012-09) */

/*--------------------------------------------------------------------------
 ITU-T Annex B (ex G.722-SWB) Source Code
 Software Release 1.00 (2010-09)
 (C) 2010 France Telecom, Huawei Technologies, NTT, VoiceAge Corp.
--------------------------------------------------------------------------*/

/*
 *------------------------------------------------------------------------
 *  File: prehpf.h
 *  Function: Header of pre-processing 1-tap high-pass filtering
 *------------------------------------------------------------------------
 */

#ifndef FPREHPF_H
#define FPREHPF_H

void  *highpass_1tap_iir_const (void);
void  highpass_1tap_iir_dest (void*);
void  highpass_1tap_iir_reset (void*);
void  highpass_1tap_iir (Word16, Word16, Word16*, Word16*, void*);

#endif  /* FPREHPF_H */
