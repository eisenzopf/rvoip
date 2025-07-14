/* ITU G.722 3rd Edition (2012-09) */

/*--------------------------------------------------------------------------
 ITU-T Annex B (ex G.722-SWB) Source Code
 Software Release 1.00 (2010-09)
 (C) 2010 France Telecom, Huawei Technologies, NTT, VoiceAge Corp.
--------------------------------------------------------------------------*/

#ifndef __ROM_FX_H__
#define __ROM_FX_H__

#include "stl.h"

#define NB_LDQ3   9
#define NB_LDQ4   27

/* RE8 Constants */
#define NB_SPHERE 32
#define NB_LEADER 36

/* AVQ Constant */
#define QR        32768

/* RE8 lattice quantiser tables */
extern const Word16 A3_[], A4_[];
extern const Word16 Select_table22[5][9];
extern const Word16 Vals_a[36][3];
extern const Word16 Vals_q[36][4];
extern const UWord16 IS_new[];

extern const Word16 DIV_mult[];
extern const Word16 DIV_shift[];

extern const UWord16 I3_[], I4_[];

extern const Word16 Da_nq_[];
extern const Word16 Da_pos_[], Da_nb_[];
extern Word16 Da_id_[];

/* swb_avq_decode.c/swb_avq_encode.c tables */
extern Word16 CodeBookL[]; /* Q14 */
extern Word16 CodeBookH[]; /* Q12 */
extern const Word16 T_Qua_MB_Coef[]; /* Q13 */

extern const Word16 sg0[]; /* Q14 */
extern const Word16 sgain_frac[]; /* Q14 */

extern const Word16 dentbl[]; /* Q15 */

extern const Word16 DQ_Gain_Norm[]; /* Q(15+n) */
extern const Word16 DQ_Gain_Exp[];

extern const Word32 L_Gain_In[];
extern const Word16 Gain_Out[];

extern const Word16 sgrad[];


#endif	/* __ROM_FX_H__ */
