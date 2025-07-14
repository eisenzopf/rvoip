/* ITU G.722 3rd Edition (2012-09) */

/*--------------------------------------------------------------------------
 ITU-T Annex B (ex G.722-SWB) Source Code
 Software Release 1.00 (2010-09)
 (C) 2010 France Telecom, Huawei Technologies, NTT, VoiceAge Corp.
--------------------------------------------------------------------------*/

#ifndef NS_H
#define NS_H

#define L_WINDOW         80       /* length of the LP analysis */
#define ORD_M            4        /* LP order (and # of "lags" in autocorr.c)  */
#define GAMMA1           30147    /* 0.92f in Q15 */
#define GAMMA1S4         7536     /* 0.92f/4 = 0.23 in Q15 */
#define MAX_NORM         16       /* when to begin noise shaping deactivation  */
/* use MAX_NORM = 32 to disable this feature */
typedef struct {
  Word16   buffer[40];            /* buffer for past decoded signal */
  Word16   mem_wfilter[ORD_M];    /* buffer for the weighting filter */
  Word16   mem_t[ORD_M];
  Word16   mem_el0[ORD_M];
  Word16   gamma;
} noiseshaping_state;

#define ORD_MP1          5        /* LP order + 1  */
#define ORD_MM1          3        /* LP order - 1  */

Word16  AutocorrNS(Word16 x[], Word16 r[], Word16 r_l[]);

/* Tables used in AutocorrNS() */
extern Word16 NS_window[L_WINDOW];
extern const Word16 NS_lag_h[ORD_M];
extern const Word16 NS_lag_l[ORD_M];

#endif
