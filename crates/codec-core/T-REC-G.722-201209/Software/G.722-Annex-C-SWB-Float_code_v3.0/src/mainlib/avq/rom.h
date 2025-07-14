/* ITU G.722 3rd Edition (2012-09) */

/*--------------------------------------------------------------------------
 ITU-T G.722 Annex C (ex G.722-SWB-Float) Source Code
 Software Release 1.01 (2012-07)
 (C) 2012 France Telecom, Huawei Technologies, VoiceAge Corp., NTT.
--------------------------------------------------------------------------*/

#ifndef __ROM_FX_H__
#define __ROM_FX_H__

#include "floatutil.h"

#define NB_LDQ3   9
#define NB_LDQ4   27

/* RE8 Constants */
#define NB_SPHERE 32
#define NB_LEADER 36

/* AVQ Constant */
#define QR        32768

/* RE8 lattice quantiser tables */
extern const Short A3_[], A4_[];
extern const Short Select_table22[5][9];
extern const Short Vals_a[36][3];
extern const Short Vals_q[36][4];
extern const unsigned short IS_new[];

extern const Short DIV_mult[];
extern const Short DIV_shift[];

extern const unsigned short I3_[], I4_[];

extern const Short Da_nq_[];
extern const Short Da_pos_[], Da_nb_[];
extern Short Da_id_[];

/* swb_avq_decode.c/swb_avq_encode.c tables */
extern Float codebookL[];
extern Float codebookH[];

extern const Float t_qua_MB_coef[];

extern const Float Gain_In_flt[];
extern const Float Gain_Out_flt[];

extern const Float f_sg0[];
extern const Float fgain_frac[];
extern const Float f_dentbl[];

extern const Float fgrad[];

#endif	/* __ROM_FX_H__ */
