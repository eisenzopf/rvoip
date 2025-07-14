/* ITU G.722 3rd Edition (2012-09) */

/*-----------------------------------------------------------------------------------
 ITU-T G.722-SWBS / G.722 Annex D - Reference C code for fixed-point implementation          
 Version 1.0
 Copyright (c) 2012,
 Huawei Technologies, France Telecom
-----------------------------------------------------------------------------------*/

/*
 *------------------------------------------------------------------------
 *  File: stereo_tools.h
 *  Function: Header of basic stereo functions and stereo bitstream functions
 *------------------------------------------------------------------------
 */

#ifndef G722TOOLS_H
#define G722TOOLS_H

#ifdef LAYER_STEREO
#include "stl.h"

/* interleaving and deinterleaving functions */
void interleave(        Word16* left,   /* i: Left input channel */
                        Word16* right,  /* i: Right input channel */
                        Word16* output, /* o: Interleaved output signal */
                        Word16  N);     /* Number of samples in input frames */

void deinterleave(const Word16* input,  /* i: Interleaved input signal */
                        Word16* left,   /* o: Left output channel */
                        Word16* right,  /* o: Right output channel */
                        Word16  N);     /* Number of samples in input frame */

void OLA(Word16 *cur_real, Word16 *mem_real, Word16 *cur_ola);
void windowStereo(Word16 *input, Word16 *mem_input, Word16 *output);

/* bitstream writing */
void write_index1(Word16* bpt_stereo, Word16 index);
void write_index2(Word16* bpt_stereo, Word16 index);
void write_index3(Word16* bpt_stereo, Word16 index);
void write_index4(Word16* bpt_stereo, Word16 index);
void write_index5(Word16* bpt_stereo, Word16 index);

/* bitstream reading */
void read_index1(Word16* bpt_stereo, Word16* index);
void read_index2(Word16* bpt_stereo, Word16* index);
void read_index3(Word16* bpt_stereo, Word16* index);
void read_index4(Word16* bpt_stereo, Word16* index);
void read_index5(Word16* bpt_stereo, Word16* index);

void zero32(Word16  n, Word32* sx);

#endif /* LAYER_STEREO */
#endif /* G722TOOLS_H */
