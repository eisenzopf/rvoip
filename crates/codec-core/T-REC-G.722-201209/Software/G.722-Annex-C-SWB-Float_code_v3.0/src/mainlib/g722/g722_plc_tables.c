/* ITU G.722 3rd Edition (2012-09) */

/*--------------------------------------------------------------------------
 ITU-T G.722 Annex C (ex G.722-SWB-Float) Source Code
 Software Release 1.01 (2012-07)
 (C) 2012 France Telecom, Huawei Technologies, VoiceAge Corp., NTT.
--------------------------------------------------------------------------*/

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

const Short    G722PLC_lag_h[ORD_LPC] = {
  32728,
  32619,
  32438,
  32187,
  31867,
  31480,
  31029,
  30517,
};

const Short    G722PLC_lag_l[ORD_LPC] = {
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
round_fx(w1*32767)
*/
const Short    G722PLC_lpc_win_80[80] = {
  (Short)  2621, (Short)  2637, (Short)  2684, (Short)  2762, (Short)  2871, 
  (Short)  3010, (Short)  3180, (Short)  3380, (Short)  3610, (Short)  3869, 
  (Short)  4157, (Short)  4473, (Short)  4816, (Short)  5185, (Short)  5581, 
  (Short)  6002, (Short)  6447, (Short)  6915, (Short)  7406, (Short)  7918, 
  (Short)  8451, (Short)  9002, (Short)  9571, (Short) 10158, (Short) 10760, 
  (Short) 11376, (Short) 12005, (Short) 12647, (Short) 13298, (Short) 13959, 
  (Short) 14628, (Short) 15302, (Short) 15982, (Short) 16666, (Short) 17351, 
  (Short) 18037, (Short) 18723, (Short) 19406, (Short) 20086, (Short) 20761, 
  (Short) 21429, (Short) 22090, (Short) 22742, (Short) 23383, (Short) 24012, 
  (Short) 24629, (Short) 25231, (Short) 25817, (Short) 26386, (Short) 26938, 
  (Short) 27470, (Short) 27982, (Short) 28473, (Short) 28941, (Short) 29386, 
  (Short) 29807, (Short) 30203, (Short) 30573, (Short) 30916, (Short) 31231, 
  (Short) 31519, (Short) 31778, (Short) 32008, (Short) 32208, (Short) 32378, 
  (Short) 32518, (Short) 32627, (Short) 32705, (Short) 32751, (Short) 32767, 
  (Short) 32029, (Short) 29888, (Short) 26554, (Short) 22352, (Short) 17694, 
  (Short) 13036, (Short)  8835, (Short)  5500, (Short)  3359, (Short)  2621
};

/* FIR decimation filter coefficients
8th order FIRLS 8000 400 900 3 19 */ 
const Short    G722PLC_fir_lp[FEC_L_FIR_FILTER_LTP] = {
  (Short)  3692, (Short)  6190, (Short)  8525, (Short) 10186, 
  (Short) 10787, (Short) 10186, (Short)  8525, (Short)  6190, (Short)  3692
};

/* High-pass filter coefficients
y[i] =      x[i]   -         x[i-1] 
+ 123/128*y[i-1]  */

/*HP 100 Hz*/
const Short G722PLC_b_hp156[2] = {31456, -31456}; /*0.96, -0.96*/
const Short G722PLC_a_hp156[2] = {32767,  28835}; /*1, 0.88*/

/*HP 50 Hz*/
const Short G722PLC_b_hp[2] = {32767, -32767}; /*1, -1*/
const Short G722PLC_a_hp[2] = {32767,  31488}; /*1, 0.96*/

const Short G722PLC_gamma_az[9] = {32767, GAMMA_AZ1, GAMMA_AZ2, GAMMA_AZ3, GAMMA_AZ4,
                                    GAMMA_AZ5, GAMMA_AZ6, GAMMA_AZ7, GAMMA_AZ8}; /*1, 0.99*/



const Float    f_G722PLC_lag[ORD_LPC] = {
(Float)0.99879041, (Float)0.99546898, (Float)0.98995779, (Float)0.98229336, 
(Float)0.97252621, (Float)0.96072037, (Float)0.94695265, (Float)0.93131180
};

const Float    f_G722PLC_lpc_win_80[80] = {
(Float)0.08000000, (Float)0.08047671, (Float)0.08190585, (Float)0.08428446, 
(Float)0.08760762, (Float)0.09186842, (Float)0.09705805, (Float)0.10316574, 
(Float)0.11017883, (Float)0.11808280, (Float)0.12686126, (Float)0.13649600, 
(Float)0.14696707, (Float)0.15825277, (Float)0.17032969, (Float)0.18317281, 
(Float)0.19675550, (Float)0.21104963, (Float)0.22602555, (Float)0.24165224, 
(Float)0.25789729, (Float)0.27472705, (Float)0.29210663, (Float)0.31000000, 
(Float)0.32837008, (Float)0.34717880, (Float)0.36638717, (Float)0.38595538, 
(Float)0.40584287, (Float)0.42600842, (Float)0.44641023, (Float)0.46700603, 
(Float)0.48775311, (Float)0.50860849, (Float)0.52952893, (Float)0.55047107, 
(Float)0.57139151, (Float)0.59224689, (Float)0.61299397, (Float)0.63358977, 
(Float)0.65399158, (Float)0.67415713, (Float)0.69404462, (Float)0.71361283, 
(Float)0.73282120, (Float)0.75162992, (Float)0.77000000, (Float)0.78789337, 
(Float)0.80527295, (Float)0.82210271, (Float)0.83834776, (Float)0.85397445, 
(Float)0.86895037, (Float)0.88324450, (Float)0.89682719, (Float)0.90967031, 
(Float)0.92174723, (Float)0.93303293, (Float)0.94350400, (Float)0.95313874, 
(Float)0.96191720, (Float)0.96982117, (Float)0.97683426, (Float)0.98294195, 
(Float)0.98813158, (Float)0.99239238, (Float)0.99571554, (Float)0.99809415, 
(Float)0.99952329, (Float)1.00000000, (Float)0.97748600, (Float)0.91214782, 
(Float)0.81038122, (Float)0.68214782, (Float)0.54000000, (Float)0.39785218, 
(Float)0.26961878, (Float)0.16785218, (Float)0.10251400, (Float)0.08000000, 
};


const Float    f_G722PLC_fir_lp[FEC_L_FIR_FILTER_LTP] = {
   (Float)0.05634018476418,   (Float)0.09445500104261,   (Float)0.13008546921365,   
   (Float)0.15542763269087,   (Float)0.16460243764646,   (Float)0.15542763269087,
   (Float)0.13008546921365,   (Float)0.09445500104261,   (Float)0.05634018476418
};

const Float f_G722PLC_b_hp156[2] = {(Float)0.96, -(Float)0.96};
const Float f_G722PLC_a_hp156[2] = {1,  (Float)0.88};

const Float f_G722PLC_b_hp[2] = {1, -1};
const Float f_G722PLC_a_hp[2] = {1,  (Float)0.96};

const Float f_G722PLC_gamma_az[9] = {1, (Float)0.99, (Float)0.9801, (Float)0.9703, (Float)0.9606,
                                    (Float)0.951, (Float)0.9415, (Float)0.9321, (Float)0.9227};



