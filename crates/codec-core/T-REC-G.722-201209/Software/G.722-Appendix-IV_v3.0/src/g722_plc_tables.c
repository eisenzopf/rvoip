/* ITU G.722 3rd Edition (2012-09) */

/* ITU-T G722 PLC Appendix IV - Reference C code for fixed-point implementation */
/* Version:       1.2                                                           */
/* Revision Date: Jul 3, 2007                                                   */

/*
   ITU-T G.722 PLC Appendix IV   ANSI-C Source Code
   Copyright (c) 2006-2007
   France Telecom
*/

#include "stl.h"
#include "g722_plc.h"

/**************
 * PLC TABLES *
 **************/

/*-----------------------------------------------------*
 | Table of lag_window for autocorrelation.            |
 | noise floor = 1.0001   = (0.9999  on r[1] ..r[10])  |
 | Bandwidth expansion = 60 Hz                         |
 |                                                     |
 | Special double precision format. See "oper_32b.c"   |
 |                                                     |
 | lag_wind[0] =  1.00000000    (not stored)           |
 | lag_wind[1] =  0.99879038                           |
 | lag_wind[2] =  0.99546897                           |
 | lag_wind[3] =  0.98995781                           |
 | lag_wind[4] =  0.98229337                           |
 | lag_wind[5] =  0.97252619                           |
 | lag_wind[6] =  0.96072036                           |
 | lag_wind[7] =  0.94695264                           |
 | lag_wind[8] =  0.93131179                           |
 |                                                     |
 | exp(-2*(pi*60*k/8000).^2)/1.0001                    |
 -----------------------------------------------------*/

const Word16    G722PLC_lag_h[ORD_LPC] = {
  32728,
  32619,
  32438,
  32187,
  31867,
  31480,
  31029,
  30517,
};

const Word16    G722PLC_lag_l[ORD_LPC] = {
  11904,
  17280,
  30720,
  25856,
  24192,
  28992,
  24384,
  7360,
};

/* LPC analysis windows
l1 = 70;
l2 = 10;
for i = 1 : l1
   n = i - 1;
   w1(i) = 0.54 - 0.46 * cos(n * pi / (l1 - 1));
end
for i = (l1 + 1) : (l1 + l2)
   w1(i) = 0.54 + 0.46 * cos((i - l1) * pi / (l2));
end
round(w1*32767)
*/
const Word16    G722PLC_lpc_win_80[80] = {
(Word16)  2621, (Word16)  2637, (Word16)  2684, (Word16)  2762, (Word16)  2871, 
(Word16)  3010, (Word16)  3180, (Word16)  3380, (Word16)  3610, (Word16)  3869, 
(Word16)  4157, (Word16)  4473, (Word16)  4816, (Word16)  5185, (Word16)  5581, 
(Word16)  6002, (Word16)  6447, (Word16)  6915, (Word16)  7406, (Word16)  7918, 
(Word16)  8451, (Word16)  9002, (Word16)  9571, (Word16) 10158, (Word16) 10760, 
(Word16) 11376, (Word16) 12005, (Word16) 12647, (Word16) 13298, (Word16) 13959, 
(Word16) 14628, (Word16) 15302, (Word16) 15982, (Word16) 16666, (Word16) 17351, 
(Word16) 18037, (Word16) 18723, (Word16) 19406, (Word16) 20086, (Word16) 20761, 
(Word16) 21429, (Word16) 22090, (Word16) 22742, (Word16) 23383, (Word16) 24012, 
(Word16) 24629, (Word16) 25231, (Word16) 25817, (Word16) 26386, (Word16) 26938, 
(Word16) 27470, (Word16) 27982, (Word16) 28473, (Word16) 28941, (Word16) 29386, 
(Word16) 29807, (Word16) 30203, (Word16) 30573, (Word16) 30916, (Word16) 31231, 
(Word16) 31519, (Word16) 31778, (Word16) 32008, (Word16) 32208, (Word16) 32378, 
(Word16) 32518, (Word16) 32627, (Word16) 32705, (Word16) 32751, (Word16) 32767, 
(Word16) 32029, (Word16) 29888, (Word16) 26554, (Word16) 22352, (Word16) 17694, 
(Word16) 13036, (Word16)  8835, (Word16)  5500, (Word16)  3359, (Word16)  2621
};


/* FIR decimation filter coefficients (in Q16)
   8th order FIRLS 8000 400 900 3 19 */ 
const Word16    G722PLC_fir_lp[FEC_L_FIR_FILTER_LTP] = {
  (Word16)  3692, (Word16)  6190, (Word16)  8525, (Word16) 10186, 
  (Word16) 10787, (Word16) 10186, (Word16)  8525, (Word16)  6190, (Word16)  3692
};

/* High-pass filter coefficients (in Q14)
   y[i] =      x[i]   -         x[i-1] 
                       + 123/128*y[i-1]  */
const Word16 G722PLC_b_hp[2] = {16384, -16384};
const Word16 G722PLC_a_hp[2] = {16384,  15744};


