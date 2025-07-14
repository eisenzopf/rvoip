/* ITU G.722 3rd Edition (2012-09) */

/* ITU-T G.722 Appendix III                                                      */
/* Version:       1.0                                                            */
/* Revision Date: Nov.02, 2006                                                   */

/*
  ITU-T G.722 Appendix III ANSI-C Source Code
  Copyright (c)  Broadcom Corporation 2006.
*/

#define MIN_UNSTBL 16         /* don't use these first G722 samples */

/*-----------------------------------------------------------------------------
 * Function: testrpc()
 *
 * Description: Test to determine if the last good frame before erasure or the
 *              first good frame after erasure is completely unvoiced or 
 *              noise.  If either one is, return a flag=0 indicating that 
 *              rephasing and time warping should not be done.  Otherwise
 *              return a flag=1 indicating that rephasing and time warping 
 *              can be done.
 *
 * Inputs:  merit - figure of merit for the last good frame
 *          *inbuf- pointer to buffer containing the first good frame speech
 *                  of length FRSZ/2 (8kHz sampling).
 *
 * Outputs: return() - flag = 0 : last good frame or first good frame is 
 *                                unvoiced.
 *                   - flag = 1 : last good frame and first good frame are
 *                                not unvoiced.
 *---------------------------------------------------------------------------*/
int testrpc(short merit, short *inbuf);

/*-----------------------------------------------------------------------------
 * Function: ppchange()
 *
 * Description: Compute the lag offset between an extrapolated signal based on
 *              the output history buffer, and the first good frame.
 *
 * Inputs:  *xq   - pointer to the history buffer
 *          pp    - pitch period to be used for PWE.
 *          *inbuf- pointer to the first good frame 16kHz data
 *          estlag- current lag estimate
 *
 * Outputs: (int) - the refined lag.
 *---------------------------------------------------------------------------*/
int ppchange( short *xq, short pp, short *inbuf);

/*-----------------------------------------------------------------------------
 * Function: resample()
 *
 * Description: Low complexity resampler.  The input buffer is stretched or
 *              shrunk by "delta" samples.  The resampling is done by a 
 *              sample shift overlap-add process.  The resulting signal is 
 *              placed in the output buffer.  In the case of stretching
 *              (delta > 0), any extra samples beyond FRSZ are not computed.
 *              The extra samples to the "left" are the ones thrown out.
 *
 * Inputs:  *in   - pointer to 16kHz input buffer
 *          *out  - pointer to buffer for output
 *          delta - number of samples to stretch (+) or shrink (-)
 *
 * Outputs: *out  - "resampled" signal.
 *---------------------------------------------------------------------------*/
void resample(Word16 *in, Word16 *out, Word16 delta);

/*-----------------------------------------------------------------------------
 * Function: refinelag()
 *
 * Description: Refine the estimated lag using only the data within the OLA 
 *              window.  Estimate the position of the OLA window by the current
 *              value of the lag.
 *
 * Inputs:  *xq   - pointer to the history buffer
 *          pp    - pitch period to be used for PWE.
 *          *inbuf- pointer to the first good frame 16kHz data
 *          estlag- current lag estimate
 *
 * Outputs: (int) - the refined lag.
 *---------------------------------------------------------------------------*/
int refinelag( short *xq, short pp, short *inbuf, short estlag);

