/* ITU G.722 3rd Edition (2012-09) */

/*--------------------------------------------------------------------------
 ITU-T G.722 Annex C (ex G.722-SWB-Float) Source Code
 Software Release 1.01 (2012-07)
 (C) 2012 France Telecom, Huawei Technologies, VoiceAge Corp., NTT.
--------------------------------------------------------------------------*/

#ifndef NS_H
#define NS_H

#define L_WINDOW         80       /* length of the LP analysis */
#define ORD_M            4        /* LP order (and # of "lags" in autocorr.c)  */
#define GAMMA1           30147    /* 0.92f in Q15 */
#define GAMMA1S4         7536     /* 0.92f/4 = 0.23 in Q15 */
#define MAX_NORM         16       /* when to begin noise shaping deactivation  */

#define FL_GAMMA1           (Float)0.92     /* 0.92f in Q15 */
#define FL_GAMMA1S4         (Float)0.23      /* 0.92f/4 = 0.23 in Q15 */
#define MAX_NORM         16       /* when to begin noise shaping deactivation  */
/* use MAX_NORM = 32 to disable this feature */
typedef struct {
  Float   buffer[40];            /* buffer for past decoded signal */
  Float   mem_wfilter[ORD_M];    /* buffer for the weighting filter */
  Float   mem_t[ORD_M];
  Float   mem_el0[ORD_M];
  Float   gamma;
} fl_noiseshaping_state;


#define ORD_MP1          5        /* LP order + 1  */
#define ORD_MM1          3        /* LP order - 1  */

Short  fl_AutocorrNS(Float x[], Float r[]);


/* Tables used in AutocorrNS() */
extern Short  NS_window[L_WINDOW];
extern const Short  NS_lag_h[ORD_M];
extern const Short  NS_lag_l[ORD_M];
extern Float fl_NS_window[L_WINDOW];
extern const Float fl_NS_lag[ORD_M];


#endif
