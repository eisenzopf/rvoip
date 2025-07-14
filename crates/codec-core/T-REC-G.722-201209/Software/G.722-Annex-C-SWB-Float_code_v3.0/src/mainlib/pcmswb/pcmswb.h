/* ITU G.722 3rd Edition (2012-09) */

/*--------------------------------------------------------------------------
 ITU-T G.722 Annex C (ex G.722-SWB-Float) Source Code
 Software Release 1.01 (2012-07)
 (C) 2012 France Telecom, Huawei Technologies, VoiceAge Corp., NTT.
--------------------------------------------------------------------------*/

#ifndef PCMSWB_H
#define PCMSWB_H

/*------------------------------------------------------------------------*
* Defines
*------------------------------------------------------------------------*/
#define MODE_R00wm   0 /* G.722        ,  WB,  48k */
#define MODE_R0wm    1 /* G.722        ,  WB,  56k */
#define MODE_R1wm    2 /* G.722        ,  WB,  64k */
#define MODE_R1sm    3 /* G.722        , SWB,  64k [R0wm+8k] */
#define MODE_R2sm    4 /* G.722        , SWB,  80k [R1wm+16k] */
#define MODE_R3sm    5 /* G.722, SWB,  96k [R1wm+16k*2,R2wm+16k] */

#define NBITS_MODE_R00wm 240 /* G.722      , WB, 48k */
#define NBITS_MODE_R0wm  280 /* G.722      , WB, 56k */
#define NBITS_MODE_R1wm  320 /* G.722      , WB, 64k */
#define NBITS_MODE_R1sm  320 /* G.722      ,SWB, 64k [R0wm+8k] */
#define NBITS_MODE_R2sm  400 /* G.722      ,SWB, 80k [R1wm+16k] */
#define NBITS_MODE_R3sm  480 /* G.722      ,SWB, 96k [R2wm+16k/R1wm+16k*2] */

#define  NSamplesPerFrame08k  40   /* Number of samples a frame in 8kHz  */
#define  NSamplesPerFrame16k  80   /* Number of samples a frame in 16kHz */
#define  NSamplesPerFrame32k 160   /* Number of samples a frame in 32kHz */

#define  NBytesPerFrame_G722_48k     30   /* G.722 48k mode */
#define  NBytesPerFrame_G722_56k     35   /* G.722 56k mode */
#define  NBytesPerFrame_G722_64k     40   /* G.722 64k mode */
#define  NBytesPerFrame_SWB_0         5   /* SWB Subcodec 0 */
#define  NBytesPerFrame_SWB_1        10   /* SWB Subcodec 1 */
#define  NBytesPerFrame_SWB_2        10   /* SWB Subcodec 2 */

#define  NBitsPerFrame_G722_48k      (NBytesPerFrame_G722_48k*8)	
#define  NBitsPerFrame_G722_56k      (NBytesPerFrame_G722_56k*8)	
#define  NBitsPerFrame_G722_64k      (NBytesPerFrame_G722_64k*8)
#define  NBitsPerFrame_SWB_0         (NBytesPerFrame_SWB_0*8)	
#define  NBitsPerFrame_SWB_1         (NBytesPerFrame_SWB_1*8)	
#define  NBitsPerFrame_SWB_2         (NBytesPerFrame_SWB_2*8)	

#define  NBYTEPERFRAME_MAX   NBytesPerFrame0 /* Max value of NBytesPerFrameX */

#define  MaxBytesPerFrame  (NBytesPerFrame_G722_48k+NBytesPerFrame_G722_56k+NBytesPerFrame_G722_64k+NBytesPerFrame_SWB_1+NBytesPerFrame_SWB_2)	
#define  MaxBitsPerFrame   (MaxBytesPerFrame*8)	

#define  L_DELAY_COMP_MAX  250 /* need to be considered */

#define NBitsPerFrame_EL1 40
#define NBitsPerFrame_SWBL2   40

#define G722EL1_MODE 2 /*1: 16kbit/s, 2:8kbit/s*/

#define NTAP_QMF_G722    24
#define QMF_DELAY_G722   (NTAP_QMF_G722-2)
#define QMF_DELAY_WB     (QMF_DELAY_G722)	

#define NTAP_QMF_SWB     32
#define QMF_DELAY_SWB    (NTAP_QMF_SWB-2)

/*------------------------------------------------------------------------*
* Prototypes
*------------------------------------------------------------------------*/
void* pcmswbEncode_const(unsigned short sampf, int mode);
void  pcmswbEncode_dest(void* p_work);
int   pcmswbEncode_reset(void* p_work);
int   pcmswbEncode( const short* inwave, unsigned char* bitstream, void* p_work );

void* pcmswbDecode_const(int mode);
void  pcmswbDecode_dest(void* p_work);
int   pcmswbDecode_reset(void* p_work);
int   pcmswbDecode( const unsigned char* bitstream, short* outwave, void* p_work, int ploss_status );
int   pcmswbDecode_set(int  mode, void*  p_work);

#endif  /* PCMSWB_H */
