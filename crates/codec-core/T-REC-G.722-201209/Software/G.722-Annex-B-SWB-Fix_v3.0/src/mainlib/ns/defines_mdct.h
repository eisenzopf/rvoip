/* ITU G.722 3rd Edition (2012-09) */

/*--------------------------------------------------------------------------
 ITU-T Annex B (ex G.722-SWB) Source Code
 Software Release 1.00 (2010-09)
 (C) 2010 France Telecom, Huawei Technologies, NTT, VoiceAge Corp.
--------------------------------------------------------------------------*/

/*
 *------------------------------------------------------------------------
 *  File: defines_mdct.h
 *  Function: MDCT constants
 *------------------------------------------------------------------------
 */

#ifndef DEFINES_MDCT_H
#define DEFINES_MDCT_H

/* constants for MDCT and inverse MDCT */
#define MDCT_L_WIN    80
#define MDCT_L_WIN2   40
#define MDCT_L_WIN4   20
#define MDCT_NP        5
#define MDCT_EXP_NPP   2
#define MDCT_NB_REV    1
#define MDCT_NPP     (1<<MDCT_EXP_NPP)

#endif /* DEFINES_MDCT_H */
