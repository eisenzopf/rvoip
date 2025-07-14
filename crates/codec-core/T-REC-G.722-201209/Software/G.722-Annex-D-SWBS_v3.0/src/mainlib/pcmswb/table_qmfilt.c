/* ITU G.722 3rd Edition (2012-09) */

/*--------------------------------------------------------------------------
 ITU-T Annex B (ex G.722-SWB) Source Code
 Software Release 1.00 (2010-09)
 (C) 2010 France Telecom, Huawei Technologies, NTT, VoiceAge Corp.
--------------------------------------------------------------------------*/

/*
 *------------------------------------------------------------------------
 *  File: table_qmfilt.c
 *  Function: Tables for Quadrature mirror filter (QMF)
 *------------------------------------------------------------------------
 */

#include "pcmswb_common.h"
#include "qmfilt.h"

const Word16 sSWBQmf0[NTAP_QMF_SWB/2] = {    /* Q15 */
     21,   -41,    47,    -6,  -135,   474, -1286,  4210,
  15285, -3270,  1734, -1021,   586,  -307,   136,   -44
};

const Word16 sSWBQmf1[NTAP_QMF_SWB/2] = {    /* Q15 */
    -44,   136,  -307,   586, -1021,  1734, -3270, 15285,
   4210, -1286,   474,  -135,    -6,    47,   -41,    21
};
