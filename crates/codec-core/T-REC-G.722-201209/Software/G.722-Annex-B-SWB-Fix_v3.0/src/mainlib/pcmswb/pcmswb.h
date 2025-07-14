/* ITU G.722 3rd Edition (2012-09) */

/*--------------------------------------------------------------------------
 ITU-T Annex B (ex G.722-SWB) Source Code
 Software Release 1.00 (2010-09)
 (C) 2010 France Telecom, Huawei Technologies, NTT, VoiceAge Corp.
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

/*------------------------------------------------------------------------*
* Prototypes
*------------------------------------------------------------------------*/
void* pcmswbEncode_const(UWord16 sampf, Word16 mode);
void  pcmswbEncode_dest(void* p_work);
Word16   pcmswbEncode_reset(void* p_work);
Word16   pcmswbEncode( const Word16* inwave, unsigned char* bitstream, void* p_work );
void* pcmswbDecode_const(Word16 mode);
void  pcmswbDecode_dest(void* p_work);
Word16   pcmswbDecode_reset(void* p_work);
Word16   pcmswbDecode( const unsigned char* bitstream, Word16* outwave, void* p_work, Word16 ploss_status );
Word16   pcmswbDecode_set(Word16  mode, void*  p_work);

#endif  /* PCMSWB_H */
