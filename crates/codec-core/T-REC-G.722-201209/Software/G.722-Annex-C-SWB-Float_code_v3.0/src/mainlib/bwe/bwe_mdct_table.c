/* ITU G.722 3rd Edition (2012-09) */

/*--------------------------------------------------------------------------
 ITU-T G.722 Annex C (ex G.722-SWB-Float) Source Code
 Software Release 1.01 (2012-07)
 (C) 2012 France Telecom, Huawei Technologies, VoiceAge Corp., NTT.
--------------------------------------------------------------------------*/

#include "bwe_mdct.h"

/***********************************************/
#define MDCT2_SBARYSZ (1 << (MDCT2_EXP_NPP-1))	

/* Index mapping table for Good-Thomas FFT */
const Short MDCT_tab_map_swbs[MDCT2_NP*MDCT2_NPP] = {	
  (Short)     0, (Short)    25, (Short)    10, (Short)    35, (Short)    20, (Short)     5, (Short)    30, (Short)    15,
  (Short)    16, (Short)     1, (Short)    26, (Short)    11, (Short)    36, (Short)    21, (Short)     6, (Short)    31,
  (Short)    32, (Short)    17, (Short)     2, (Short)    27, (Short)    12, (Short)    37, (Short)    22, (Short)     7,
  (Short)     8, (Short)    33, (Short)    18, (Short)     3, (Short)    28, (Short)    13, (Short)    38, (Short)    23,
  (Short)    24, (Short)     9, (Short)    34, (Short)    19, (Short)     4, (Short)    29, (Short)    14, (Short)    39
};

/* Index mapping table for Good-Thomas FFT */
const Short MDCT_tab_map2_swbs[MDCT2_NP*MDCT2_NPP] = {	
  (Short)     0, (Short)     5, (Short)    10, (Short)    15, (Short)    20, (Short)    25, (Short)    30, (Short)    35,
  (Short)     8, (Short)    13, (Short)    18, (Short)    23, (Short)    28, (Short)    33, (Short)    38, (Short)     3,
  (Short)    16, (Short)    21, (Short)    26, (Short)    31, (Short)    36, (Short)     1, (Short)     6, (Short)    11,
  (Short)    24, (Short)    29, (Short)    34, (Short)    39, (Short)     4, (Short)     9, (Short)    14, (Short)    19,
  (Short)    32, (Short)    37, (Short)     2, (Short)     7, (Short)    12, (Short)    17, (Short)    22, (Short)    27,
};

/* FFT twiddle factors (cosine part) */
const Float MDCT_rw1_tbl_swbf[MDCT2_SBARYSZ] = {  
 (Float)  0.999969482421875f/*1.0f*/, (Float) 0.70709228515625f, (Float)     0.0f, (Float)-0.70709228515625f
};

/* FFT twiddle factors (sine part) */
const Float MDCT_rw2_tbl_swbf[MDCT2_SBARYSZ] = {      
  (Float)     0.0f, (Float) 0.70709228515625f, (Float) 0.999969482421875f/*1.0f*/, (Float) 0.70709228515625f 
};

/* Table for Good-Thomas FFT */
const Short MDCT_tab_rev_ipp_swbs[MDCT2_NB_REV] = { 
  (Short)     1, (Short)     3 
};

/* Table for Good-Thomas FFT */
const Short MDCT_tab_rev_i_swbs[MDCT2_NB_REV] = { 
  (Short)     4, (Short)     6 
};

/* Cosine table for FFT */
const Float   MDCT_xcos_swbf[MDCT2_NP * MDCT2_NP] = {
  (Float) 1.000000000e+000, (Float) 1.000000000e+000,
  (Float) 1.000000000e+000, (Float) 1.000000000e+000,
  (Float) 1.000000000e+000,
  (Float) 1.000000000e+000, (Float) 3.090169944e-001,
  (Float)-8.090169944e-001, (Float)-8.090169944e-001,
  (Float) 3.090169944e-001,
  (Float) 1.000000000e+000, (Float)-8.090169944e-001,
  (Float) 3.090169944e-001, (Float) 3.090169944e-001,
  (Float)-8.090169944e-001,
  (Float) 1.000000000e+000, (Float)-8.090169944e-001,
  (Float) 3.090169944e-001, (Float) 3.090169944e-001,
  (Float)-8.090169944e-001,
  (Float) 1.000000000e+000, (Float) 3.090169944e-001,
  (Float)-8.090169944e-001, (Float)-8.090169944e-001,
  (Float) 3.090169944e-001
};

/* Sine table for FFT */
const Float   MDCT_xsin_swbf[MDCT2_NP * MDCT2_NP] = {
  (Float) 0.000000000e+000, (Float) 0.000000000e+000,
  (Float) 0.000000000e+000, (Float) 0.000000000e+000,
  (Float) 0.000000000e+000,
  (Float) 0.000000000e+000, (Float)-9.510565163e-001,
  (Float)-5.877852523e-001, (Float) 5.877852523e-001,
  (Float) 9.510565163e-001,
  (Float) 0.000000000e+000, (Float)-5.877852523e-001,
  (Float) 9.510565163e-001, (Float)-9.510565163e-001,
  (Float) 5.877852523e-001,
  (Float) 0.000000000e+000, (Float) 5.877852523e-001,
  (Float)-9.510565163e-001, (Float) 9.510565163e-001,
  (Float)-5.877852523e-001,
  (Float) 0.000000000e+000, (Float) 9.510565163e-001,
  (Float) 5.877852523e-001, (Float)-5.877852523e-001,
  (Float)-9.510565163e-001
};

/* MDCT window */
const Float MDCT_h_swbf[MDCT2_L_WIN2] = {
  (Float) 0.01385498046875f,	(Float) 0.0416259765625f,	(Float) 0.06939697265625f,	(Float) 0.09710693359375f,	(Float) 0.12481689453125f,
  (Float) 0.15240478515625,		(Float) 0.17999267578125f,	(Float) 0.20751953125f,		(Float) 0.23492431640625f,	(Float) 0.26226806640625f,
  (Float) 0.28948974609375f,	(Float) 0.316650390625f,	(Float) 0.3436279296875f,	(Float) 0.3704833984375f,	(Float) 0.397216796875f,
  (Float) 0.42376708984375f,	(Float) 0.4501953125f,		(Float) 0.4764404296875f,	(Float) 0.50250244140625f,	(Float) 0.5283203125f,
  (Float) 0.55401611328125f,	(Float) 0.5794677734375f,	(Float) 0.60467529296875f,	(Float) 0.629638671875f,	(Float) 0.65435791015625f,
  (Float) 0.67889404296875f,	(Float) 0.703125f,			(Float) 0.72705078125f,		(Float) 0.750732421875f,	(Float) 0.77410888671875f,
  (Float) 0.79718017578125f,	(Float) 0.82000732421875f,	(Float) 0.84246826171875f,	(Float) 0.86456298828125f,	(Float) 0.88641357421875f,
  (Float) 0.9078369140625f,		(Float) 0.928955078125f,	(Float) 0.94970703125f,		(Float) 0.9700927734375f,	(Float) 0.9901123046875f,
  (Float) 1.009765625f,			(Float) 1.02899169921875f,	(Float) 1.0478515625f,		(Float) 1.0662841796875f,	(Float) 1.0843505859375f,
  (Float) 1.1019287109375f,		(Float) 1.119140625f,		(Float) 1.13592529296875f,	(Float) 1.1522216796875f,	(Float) 1.1680908203125f,
  (Float) 1.18353271484375f,	(Float) 1.198486328125f,	(Float) 1.2130126953125f,	(Float) 1.22705078125f,		(Float) 1.2406005859375f,
  (Float) 1.25372314453125f,	(Float) 1.26629638671875f,	(Float) 1.2784423828125f,	(Float) 1.2900390625f,		(Float) 1.30120849609375f,
  (Float) 1.31182861328125f,	(Float) 1.32196044921875f,	(Float) 1.33154296875f,		(Float) 1.34063720703125f,	(Float) 1.3492431640625f,
  (Float) 1.3572998046875f,		(Float) 1.36480712890625f,	(Float) 1.371826171875f,	(Float) 1.3782958984375f,	(Float) 1.38427734375f,
  (Float) 1.38970947265625f,	(Float) 1.39459228515625f,	(Float) 1.39892578125f,		(Float) 1.4027099609375f,	(Float) 1.40594482421875,
  (Float) 1.40869140625f,		(Float) 1.410888671875f,	(Float) 1.41253662109375f,	(Float) 1.41357421875f,		(Float) 1.41412353515625f
};

/* Sine table for MDCT and iMDCT */
const Float MDCT_wsin_swbf[MDCT2_L_WIN4+1] = {	
  (Float) 0.0f,					(Float) 0.03924560546875f,		(Float) 0.078460693359375f,		(Float) 0.117523193359375f,		(Float) 0.15643310546875f,
  (Float) 0.195098876953125f,	(Float) 0.23345947265625f,		(Float) 0.271453857421875f,		(Float) 0.30902099609375f,		(Float) 0.34613037109375f,
  (Float) 0.3826904296875,		(Float) 0.418670654296875f,		(Float) 0.4539794921875f,		(Float) 0.488616943359375f,		(Float) 0.522491455078125f,
  (Float) 0.555572509765625,	(Float) 0.587799072265625f,		(Float) 0.61907958984375f,		(Float) 0.649444580078125f,		(Float) 0.678802490234375f,
  (Float) 0.70709228515625f,	(Float) 0.73431396484375f,		(Float) 0.760406494140625f,		(Float) 0.785308837890625f,		(Float) 0.80902099609375f,
  (Float) 0.83148193359375f,	(Float) 0.852630615234375f,		(Float) 0.87249755859375f,		(Float) 0.891021728515625f,		(Float) 0.90814208984375f,
  (Float) 0.92388916015625f,	(Float) 0.938201904296875f,		(Float) 0.9510498046875f,		(Float) 0.96246337890625f,		(Float) 0.972381591796875f,
  (Float) 0.982147216796875f,	(Float) 0.987701416015625f,		(Float) 0.993072509765625f,		(Float) 0.996917724609375f,		(Float) 0.999237060546875f,
  (Float) 1.0f
};

/* Table for complex post-multiplication in MDCT (real part) */
const Float MDCT_wetr_swbf[MDCT2_L_WIN4] = {
  (Float) -0.004462718963623046875f,	(Float) 0.00463104248046875f,		(Float)-0.00479221343994140625f,	(Float) 0.0049457550048828125f, 
  (Float) -0.005092144012451171875f,	(Float) 0.005230426788330078125f,	(Float)-0.00536060333251953125f,	(Float) 0.00548267364501953125f, 
  (Float) -0.005596160888671875f,		(Float) 0.005701541900634765625f,	(Float)-0.00579738616943359375f,	(Float) 0.005884647369384765625f, 
  (Float) -0.005962848663330078125f,	(Float) 0.006031513214111328125f,	(Float)-0.00609111785888671875f,	(Float) 0.00614166259765625f, 
  (Float) -0.006182193756103515625f,	(Float) 0.006213665008544921875f,	(Float)-0.0062351226806640625f,		(Float) 0.00624752044677734375f,
  (Float) -0.006249904632568359375f,	(Float) 0.006242275238037109375f,	(Float)-0.0062255859375f,			(Float) 0.006199359893798828125, 
  (Float) -0.006163120269775390625f,	(Float) 0.00611782073974609375f,	(Float)-0.00606250762939453125f,	(Float) 0.0059986114501953125f, 
  (Float) -0.005924701690673828125f,	(Float) 0.0058422088623046875f,		(Float)-0.0057506561279296875f,		(Float) 0.005650043487548828125f, 
  (Float) -0.0055408477783203125f,		(Float) 0.005423069000244140625f,	(Float)-0.0052967071533203125f,		(Float) 0.00516223907470703125f, 
  (Float) -0.0050201416015625f,			(Float) 0.004869937896728515625f,	(Float)-0.004712581634521484375f,	(Float) 0.004547595977783203125f
};


/* Table for complex post-multiplication in MDCT (imaginary part) */
const Float MDCT_weti_swbf[MDCT2_L_WIN4] = {
  (Float)  0.004375934600830078125f,	(Float) -0.00419712066650390625f,	(Float)  0.00401210784912109375f,	(Float) -0.003820896148681640625f,
  (Float)  0.00362396240234375f,		(Float) -0.003421306610107421875f,	(Float)  0.00321292877197265625f,	(Float) -0.0030002593994140625f,
  (Float)  0.0027828216552734375f,		(Float) -0.00256061553955078125f,	(Float)  0.002335071563720703125f,	(Float) -0.002105712890625f,
  (Float)  0.001873016357421875f,		(Float) -0.00163745880126953125f,	(Float)  0.001399517059326171875f,	(Float) -0.001159191131591796875f,
  (Float)  0.000916957855224609375f,	(Float) -0.000673770904541015625f,	(Float)  0.0004291534423828125f,	(Float) -0.00018405914306640625f,
  (Float) -0.000061511993408203125f,	(Float)  0.000306606292724609375f,	(Float) -0.000551700592041015625f,	(Float)  0.0007953643798828125f,
  (Float) -0.001038074493408203125f,	(Float)  0.001279354095458984375f,	(Float) -0.001518726348876953125f,	(Float)  0.001755237579345703125f,
  (Float) -0.001989841461181640625f,	(Float)  0.002220630645751953125f,	(Float) -0.002448558807373046875f,	(Float)  0.0026721954345703125f,
  (Float) -0.002892017364501953125f,	(Float)  0.0031070709228515625f,	(Float) -0.00331783294677734375f,	(Float)  0.003523349761962890625f,
  (Float) -0.00372314453125f,			(Float)  0.003917217254638671875f,	(Float) -0.00410556793212890625f,	(Float)  0.004287242889404296875f,
};

/* Table for complex pre-multiplication in iMDCT (real part) */
const Float MDCT_wetrm1_swbf[MDCT2_L_WIN4] = {
  (Float)-1.42803955078125f,	(Float) 1.48187255859375f,	(Float)-1.53350830078125f,	(Float) 1.58270263671875f,	(Float)-1.6295166015625f,
  (Float) 1.67376708984375f,	(Float)-1.7154541015625f,	(Float) 1.7545166015625f,	(Float)-1.79083251953125f,	(Float) 1.82440185546875f,
  (Float)-1.85516357421875f,	(Float) 1.88311767578125f,	(Float)-1.9080810546875f,	(Float) 1.93017578125f,		(Float)-1.94921875f,
  (Float) 1.96533203125f,		(Float)-1.97833251953125f,	(Float) 1.98834228515625f,	(Float)-1.99530029296875f,	(Float) 1.9991455078125f,
  (Float)-1.9998779296875f,		(Float) 1.99761962890625f,	(Float)-1.9921875f,			(Float) 1.98370361328125f,	(Float)-1.97222900390625f,
  (Float) 1.9576416015625f,		(Float)-1.9400634765625f,	(Float) 1.9193115234375f,	(Float)-1.89593505859375f,	(Float) 1.8695068359375f,
  (Float)-1.84014892578125f,	(Float) 1.8079833984375f,	(Float)-1.77301025390625f,	(Float) 1.73529052734375f,	(Float)-1.6949462890625f,
  (Float) 1.6519775390625f,		(Float)-1.6064453125f,		(Float) 1.55841064453125f,	(Float)-1.50799560546875f,	(Float) 1.45526123046875f
};

/* Table for complex pre-multiplication in iMDCT (imaginary part) */
const Float MDCT_wetim1_swbf[MDCT2_L_WIN4] = {
  (Float)-1.4002685546875f,		(Float) 1.3431396484375f,	(Float)-1.28387451171875f,	(Float) 1.22271728515625f,	(Float)-1.15960693359375f,
  (Float) 1.09478759765625f,	(Float)-1.0281982421875f,	(Float) 0.9600830078125f,	(Float)-0.89044189453125f,	(Float) 0.8194580078125f,
  (Float)-0.7471923828125f,		(Float) 0.67376708984375f,	(Float)-0.59930419921875f,	(Float) 0.52398681640625f,	(Float)-0.44775390625f,
  (Float) 0.37091064453125f,	(Float)-0.29345703125f,		(Float) 0.215576171875f,	(Float)-0.1373291015625f,	(Float) 0.05889892578125f,
  (Float) 0.0196533203125f,		(Float)-0.09814453125f,		(Float) 0.176513671875f,	(Float)-0.25457763671875f,	(Float) 0.332275390625f,
  (Float)-0.409423828125f,		(Float) 0.4859619140625f,	(Float)-0.561767578125f,	(Float) 0.63665771484375f,	(Float)-0.71063232421875f,
  (Float) 0.783447265625f,		(Float)-0.8551025390625f,	(Float) 0.9254150390625f,	(Float)-0.99432373046875f,	(Float) 1.06170654296875f,
  (Float)-1.12744140625f,		(Float) 1.19140625f,		(Float)-1.2535400390625f,	(Float) 1.31378173828125f,	(Float)-1.3719482421875f
};
