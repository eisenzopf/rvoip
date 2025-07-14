/* ITU G.722 3rd Edition (2012-09) */

/* ITU-T G.722 Appendix III                                                      */
/* Version:       1.0                                                            */
/* Revision Date: Nov.02, 2006                                                   */

/*
  ITU-T G.722 Appendix III ANSI-C Source Code
  Copyright (c)  Broadcom Corporation 2006.
*/

#include "stl.h"
#include "utility.h"
#include "g722plc.h"
#include "table.h"
#ifndef G722DEMO
#include "ppchange.h"
#include "g722.h"
#if DMEM
#include "memutil.h"
#endif



/* local function definitions */
void RestorePlcInfo(struct WB_PLC_State *plc);
void CopyPlcInfo(struct WB_PLC_State *plc);
void CopyG722DecoderMem(g722_state *to, g722_state *from);

/* local #defines */
#define NBPL_LOW_STAT  9830
#define NBPL_HIGH_STAT 6554
#define ONE_OVER_SLOW_MINUS_SHIGH 20485 // << 11 -> Q26


/*-----------------------------------------------------------------------------
 * Function: G722DecWithPLC()
 *
 * Description: highest level G722 decoder function for use with PLC.  This 
 *              function should be called for received frame AND lost frames.
 *
 * Inputs:  *chan    - pointer to buffer containing the channel indices for
 *                     the current frame.
 *          mode     - G722 decoder mode.
 *          blocksize- number of samples per frame - MUST be a multiple of 160.
 *          *ds      - pointer to the structure containing the current
 *                     decoder state memory.
 *          *plc     - pointer to the structure containing the current
 *                     plc state memory.
 *          bfi      - bad frame indicator (0=good frame, 1=lost frame)
 *
 * Outputs: *output  - output samples(=blocksize) written to memory starting
 *                     from this pointer.
 *---------------------------------------------------------------------------*/
short G722DecWithPLC(short *chan, short *output, short mode, short blocksize,
                 g722_state *ds, struct WB_PLC_State *plc, short bfi)
{
#if DMEM
   Word16 *qdb;
   Word16 *lb;
   Word16 *hb;
   Word16 *tt;
   Word16 *tout;
#else

   Word16 qdb[22+MAXOS];
   Word16 tt[FRSZ/2+6];
   Word16 lb[FRSZ/2+MAXOS/2];
   Word16 hb[FRSZ/2+MAXOS/2];
   Word16 tout[FRSZ/2+1];
#endif
   Word16 *ptt;

   /* Decoder variables */
   Word16 a;
   Word32 a0;

  /* Auxiliary variables */ 
   short             i, j, k;
   Word16 lag;
   Word16 *p_output, *p_qdb;

   IF (bfi)    /* BAD FRAME PROCESSING */
   {
		  
	   plc->hp_flag = 0;
#if WMOPS
      move16();
#endif

	  /* correct Q-value of excitation signals */
	  IF(plc->cfecount == 0){
        ds->rlt[1] = shl(ds->rlt[1],1);
		  ds->rh[1]  = shl(ds->rh[1],1);
		  ds->dlt[1] = shl(ds->dlt[1],1);
		  ds->dlt[2] = shl(ds->dlt[2],1);
		  ds->dlt[3] = shl(ds->dlt[3],1);
		  ds->dlt[4] = shl(ds->dlt[4],1);
		  ds->dlt[5] = shl(ds->dlt[5],1);
		  ds->dlt[6] = shl(ds->dlt[6],1);
		  ds->dh[1]  = shl(ds->dh[1],1);
		  ds->dh[2]  = shl(ds->dh[2],1);
		  ds->dh[3]  = shl(ds->dh[3],1);
		  ds->dh[4]  = shl(ds->dh[4],1);
		  ds->dh[5]  = shl(ds->dh[5],1);
		  ds->dh[6]  = shl(ds->dh[6],1);
#if WMOPS
        {
            Word16 jj;
            for (jj=0;jj<14;jj++)
               move16();
        }
#endif
	  }

      FOR (i=0;i<blocksize/FRSZ;i++)
      {
#if DMEM
         qdb = allocWord16(0, 21+MAXOS);
#endif

         /* WB PLC on Full Band Signal */
         WB_PLC_erasure( plc, &output[i*FRSZ], qdb);    
#if DMEM
         lb  = allocWord16(0, (FRSZ+MAXOS)/2-1);
         hb  = allocWord16(0, (FRSZ+MAXOS)/2-1);
#endif
         /* Use the 1st 22 samples of WB PLC output as */
         /* filter memory for the QMF TX filter */
		   FOR (j=2;j<24;j++)
		   {
			   ds->qmf_tx_delayx[j] = output[i*FRSZ+23-j];
#if WMOPS
            move16();
#endif
		   }
         p_output = output + add(22, i_mult(i,FRSZ));
		   
         /* Get the lb and hb signals by QMF TX filtering */
         FOR (k = 0; k < FRSZ/2-11; k++)
         {
            qmf_tx (p_output[1],p_output[0], &lb[k], &hb[k], ds);
            p_output += 2; 
         }

         p_qdb = qdb;
         FOR (k = FRSZ/2-11; k < (FRSZ+MAXOS)/2; k++)
         {
            /* Calculation of the synthesis QMF samples */
            qmf_tx (p_qdb[1], p_qdb[0], &lb[k], &hb[k], ds);
            p_qdb += 2;
         }
		    /* Update low- and high-band ADPCM state memory */
		   hsbupd (plc, ds, hb, (FRSZ-MAXOS)/2);
		   lsbupd (plc, ds, lb, (FRSZ-MAXOS)/2);
         CopyG722DecoderMem(&(plc->ds), ds);
#if WMOPS
         move16();
#endif
         CopyPlcInfo(plc);
		   hsbupd (plc, ds, hb+(FRSZ-MAXOS)/2, MAXOS/2);
		   lsbupd (plc, ds, lb+(FRSZ-MAXOS)/2, MAXOS/2);
         W16copy(plc->lb, lb+((FRSZ-MAXOS)/2)-11, MAXOS+11);
         W16copy(plc->hb, hb+((FRSZ-MAXOS)/2)-11, MAXOS+11);
#if DMEM
         deallocWord16(qdb, 0, 21+MAXOS);
         deallocWord16(lb, 0, (FRSZ+MAXOS)/2-1);
         deallocWord16(hb, 0, (FRSZ+MAXOS)/2-1);
#endif 
      }
   }
   ELSE     /* Good Frame Processing */
   {

	  /* First good frame after loss(es) */
	  IF(plc->ngfae == 0)
     {
#if DMEM
        tt = allocWord16(0, FRSZ/2+6-1);  
        tout = allocWord16(0, FRSZ/2+1-1); 
#endif

         /* decode the excitation and filter */               
         ptt = tt+6;
         tt[0] = shr(ds->dlt[6],1);
         tt[1] = shr(ds->dlt[5],1);
         tt[2] = shr(ds->dlt[4],1);
         tt[3] = shr(ds->dlt[3],1);
         tt[4] = shr(ds->dlt[2],1);
         tt[5] = shr(ds->dlt[1],1);

#ifdef WMOPS
         move16();move16();move16();move16();move16();move16();
#endif

         dltdec(chan, ds->detl, ds->nbl, ptt, FRSZ/2);
         filtdlt(ptt, ds, tout+1, FRSZ/2);
#if DMEM
        deallocWord16(tt, 0, (FRSZ/2)+6-1);  
#endif
        plc->lag = 0;
#ifdef WMOPS
        move16();
        test();
#endif
        /* compute the lag for rephasing and time-warping if previous and current */
        /* good frames are not unvoiced/noise */
        IF ((testrpc(plc->merit, tout+1))&&(plc->ptfe>0))
        {
            plc->lag = ppchange( plc->xq, plc->pp, tout+1);
#ifdef WMOPS
            move16();
#endif
            if (plc->lag==-100)
            {
               plc->lag=0;
#ifdef WMOPS
               move16();
#endif
            }
        }
#if DMEM
        deallocWord16(tout, 0, (FRSZ/2)+1-1);
#endif
        lag = plc->lag;
#ifdef WMOPS
        move16();
#endif
        /* RE-PHASING of memory */
        /* Update low- and high-band ADPCM state memory */
        IF (lag > 0)
        {
            RestorePlcInfo(plc);
            CopyG722DecoderMem(ds, &(plc->ds));

		      hsbupd (plc, ds, (plc->hb)+11, (short)((MAXOS-lag)/2));
		      lsbupd (plc, ds, (plc->lb)+11, (short)((MAXOS-lag)/2));
#ifdef WMOPS
            move16();
#endif
        }
        IF (lag < 0)
        {
		      hsbupd (plc, ds, (plc->hb)+11+(MAXOS/2), (short)((-lag)/2));
		      lsbupd (plc, ds, (plc->lb)+11+(MAXOS/2), (short)((-lag)/2));
        }

	  /* Update the QMF RX Filter Memory */
      j=2;
#if WMOPS
      move16();
#endif

      /* Update qmf filter memory based on the rephased position */
      FOR (k=0;k<11;k++)
      {
         ds->qmf_rx_delayx[j++] = sub (plc->lb[((int)(22+MAXOS-lag)/2)-1-k], plc->hb[((int)(22+MAXOS-lag)/2)-1-k]);
         ds->qmf_rx_delayx[j++] = add (plc->lb[((int)(22+MAXOS-lag)/2)-1-k], plc->hb[((int)(22+MAXOS-lag)/2)-1-k]);
#ifdef WMOPS
         move16();move16();
#endif
      }

		  /* Reset measure of degree of pl and ph signal above or below zero */
		  plc->pl_postn = 0;
		  plc->lb_reset = 0;
		  plc->ph_postn = 0;
		  plc->hb_reset = 0;
#if WMOPS
		  move16();move16();move16();move16();
#endif
		  
		  /* Set nbh to averag prior to erasure */
		  ds->nbh  = plc->nbph_mean;		
		  ds->deth = scaleh(ds->nbh);
#if WMOPS
		  move16();
        move16();
#endif
		  /* Set nbl depending on stationarity of nbl prior to erasure */
		  IF(sub(plc->nbpl_chng,NBPL_LOW_STAT)>0){ /* non-stationary nbl prior to loss */
			  ds->nbl = ds->nbl; /* leave at value from "re-encoding" */
#ifdef WMOPS
			  move16();
#endif
		  }
		  ELSE IF(sub(plc->nbpl_chng,NBPL_HIGH_STAT)<0){ /* stationary nbl prior to loss */
			  ds->nbl = plc->nbpl_mean2; /* set to averaged nbl */
#ifdef WMOPS
			  move16();
#endif
		  }
		  ELSE{ /* somewhere in-between stationary and non-stationary - linear interpolation */
			  a0 = L_mult(ONE_OVER_SLOW_MINUS_SHIGH, ds->nbl);
			  a0 = L_msu(a0, ONE_OVER_SLOW_MINUS_SHIGH, plc->nbpl_mean2);
			  a  = round(a0); // a in << 11
			  a0 = L_mult(plc->nbpl_mean2,2048);
			  a0 = L_msu(a0, a, NBPL_HIGH_STAT); // b in << 12
			  a0 = L_mac(a0, a, plc->nbpl_chng);
			  ds->nbl = round(L_shl(a0,4));
#if WMOPS
        move16();
#endif
		  }
		  ds->detl = scalel(ds->nbl);
#if WMOPS
        move16();
#endif		  
		  plc->nbph_lp   = plc->nbph_mean;
#ifdef WMOPS
        move16();
#endif

		  /* Set mode to re-converge nbh after lost frame(s) */
		  IF(sub(plc->nbph_chng,819) < 0)		/* highly stationary before lost frame(s) */
			  plc->nbh_mode = 2;
		  ELSE IF(sub(plc->nbph_chng,1311) < 0)	/* somewhat stationary before lost frame(s) */
			  plc->nbh_mode = 1;
		  ELSE									/* transition before lost frame(s) */
			  plc->nbh_mode = 0;
#ifdef WMOPS
        move16();
#endif
	  }

	  /* ADPCM decoding and update the PLC statemem */
      FOR (i=0;i<blocksize/FRSZ;i++)
      {
		  /* Control stabilization of HB ADPCM pole adaptation after lost frame(s) */
#ifdef WMOPS
        move16();
#endif
		  if(sub(plc->ngfae,4)<0)
			  plc->hp_flag = 1;
		  if (sub(plc->ngfae,4)>=0)
			  plc->hp_flag = 0;

		  /* Perform G.722 decode */
            g722_decode(chan+i*FRSZ/2, output+i*FRSZ, 1, FRSZ/2, ds, plc);

			/* correct Q-value of excitation signals */
			IF(plc->cfecount != 0){
            ds->rlt[1] = shr(ds->rlt[1],1);
				ds->rh[1]  = shr(ds->rh[1],1);
				ds->dlt[1] = shr(ds->dlt[1],1);
				ds->dlt[2] = shr(ds->dlt[2],1);
				ds->dlt[3] = shr(ds->dlt[3],1);
				ds->dlt[4] = shr(ds->dlt[4],1);
				ds->dlt[5] = shr(ds->dlt[5],1);
				ds->dlt[6] = shr(ds->dlt[6],1);
				ds->dh[1]  = shr(ds->dh[1],1);
				ds->dh[2]  = shr(ds->dh[2],1);
				ds->dh[3]  = shr(ds->dh[3],1);
				ds->dh[4]  = shr(ds->dh[4],1);
				ds->dh[5]  = shr(ds->dh[5],1);
				ds->dh[6]  = shr(ds->dh[6],1);
#if WMOPS
            {
               Word16 jj;
               for (jj=0;jj<14;jj++)
                  move16();
            }
#endif
			}

		  /* WB PLC on LB */
		  WB_PLC( plc, &output[FRSZ*i], &output[FRSZ*i]);
      }
   }

   return(blocksize);
}

/*-----------------------------------------------------------------------------
 * Function: CopyPlcInfo()
 *
 * Description: copy the plc state information that potentially needs to be 
 *              restored for rephasing.
 *
 * Inputs:  *plc  - pointer to plc state memory
 *
 * Outputs: *plc  - values copied internally
 *---------------------------------------------------------------------------*/
void CopyPlcInfo(struct WB_PLC_State *plc)
{
   plc->cpl_postn = plc->pl_postn;
   plc->cph_postn = plc->ph_postn;
   plc->crhhp_m1 = plc->rhhp_m1;
   plc->crh_m1 = plc->rh_m1;
   plc->cphhp_m1 = plc->phhp_m1;
   plc->cph_m1 = plc->ph_m1;
#ifdef WMOPS
   move16();move16();move16();move16();move16();move16();
#endif

}

/*-----------------------------------------------------------------------------
 * Function: RestorePlcInfo()
 *
 * Description: restore the plc state information for rephasing.
 *
 * Inputs:  *plc  - pointer to plc state memory
 *
 * Outputs: *plc  - values retored
 *---------------------------------------------------------------------------*/
void RestorePlcInfo(struct WB_PLC_State *plc)
{
   plc->pl_postn = plc->cpl_postn;
   plc->ph_postn = plc->cph_postn;
   plc->rhhp_m1 = plc->crhhp_m1;
   plc->rh_m1 = plc->crh_m1;
   plc->phhp_m1 = plc->cphhp_m1;
   plc->ph_m1 = plc->cph_m1;
#ifdef WMOPS
   move16();move16();move16();move16();move16();move16();
#endif
}

/*-----------------------------------------------------------------------------
 * Function: CopyG722DecoderMem()
 *
 * Description: Copy g722 state memory that needs to be preserved
 *
 * Inputs:  *to  - pointer to plc state memory being copied TO
 *          *from- pointer to plc state memory being copied FROM
 *
 * Outputs: *from- values copied.
 *---------------------------------------------------------------------------*/
void CopyG722DecoderMem(g722_state *to, g722_state *from)
{
   Word16 i;

   to->al[1] = from->al[1];
   to->al[2] = from->al[2];
   to->ah[1] = from->ah[1];
   to->ah[2] = from->ah[2];
   to->plt[1]= from->plt[1];
   to->plt[2]= from->plt[2];
   to->ph[1] = from->ph[1];
   to->ph[2] = from->ph[2];
   to->rlt[1]= from->rlt[1];
   to->rh[1] = from->rh[1];
#if WMOPS
      move16();move16();move16();move16();move16();
      move16();move16();move16();move16();move16();
#endif
   FOR (i=1;i<7;i++)
   {
      to->bl[i] = from->bl[i];
      to->dlt[i]= from->dlt[i];
      to->bh[i] = from->bh[i];
      to->dh[i] = from->dh[i];
#if WMOPS
      move16();move16();move16();move16();
#endif
   }
   to->detl = from->detl;
   to->nbl  = from->nbl;
   to->deth = from->deth;
   to->sl   = from->sl;
   to->szl  = from->szl;
   to->nbh  = from->nbh;
   to->sh   = from->sh;
   to->szh  = from->szh;
#if WMOPS
      move16();move16();move16();move16();
      move16();move16();move16();move16();
#endif
}



/*-----------------------------------------------------------------------------
 * Function: WB_PLC_common()
 *
 * Description: PLC function called in both good and bad frames
 *
 * Inputs:  *plc  - pointer to plc state memory
 *          *out  - pointer to output buffer
 *          *xq   - pointer to output history buffer
 *          good_frame - 0=bad, 1=good
 *
 * Outputs: *out  - potentially modified output buffer
 *---------------------------------------------------------------------------*/
void 	WB_PLC_common(
struct 	WB_PLC_State *plc,
Word16	*out,
Word16	*xq,
Word16	good_frame)
{
#if DMEM
   Word16  *xw;
   Word32  *rl;
   Word16  *xwd;
   Word16  *awl;
#else
	Word16	xw[DFO+FRSZ];
	Word32	rl[1+LPCO];
	Word16	xwd[LXD];
	Word16	awl[1+LPCO];
#endif
   Word32   a0;
	Word16	cpp;
	int		i;
   Word16   tmp16;

	/* extract from the buffer to OUTPUT */
	W16copy(out, xq+XQOFF, FRSZ);

	/* PERFORM LPC ANALYSIS WITH ASYMMETRICAL WINDOW */
	IF (good_frame) 
   {
#if DMEM
      rl = allocWord32(0, LPCO);
#endif
      Autocorr(rl,xq+LXQ-WINSZ,win,WINSZ,LPCO); /* get autocorrelation coeff. */
      Spectral_Smoothing(LPCO,rl,sstwin_h,sstwin_l);   /* spectral smoothing */
      Levinson(rl, plc->al, plc->alast, LPCO);    /* Levinson-Durbin recursion */
#if DMEM
      deallocWord32(rl, 0, LPCO);
#endif
      FOR (i=1;i<=LPCO;i++)
      {/* bandwidth expansion */
         plc->al[i] = mult_r(bwel[i],plc->al[i]);
#ifdef WMOPS
         move16();
#endif
      }
	}
#if DMEM
   xw = allocWord16(0, DFO+FRSZ-1);
#endif
	/* CALCULATE LPC PREDICTION RESIDUAL (temporarily put it in xw[] array) */
	azfilterQ0_Q1(plc->al,LPCO,xq+XQOFF,xw+DFO,FRSZ); 

	/* CALCULATE AVERAGE MAGNITUDE OF LPC PREDICTION RESIDUAL */
   IF (good_frame)
   {
	   a0 = abs_s(xw[DFO]);
	   FOR (i=1;i<FRSZ;i++) 
		   a0 = L_add(a0,(Word32)abs_s(xw[DFO+i]));
      tmp16 = (Word16) L_shr(a0, 7);/* divide by 128, Q1->Q2 */
	   plc->avm = add(tmp16, mult(tmp16, 19661));   /* avm *= 1.6 */
#ifdef WMOPS
      move16();
#endif
   }
	/* GET PERCEPTUALLY WEIGHTED VERSION OF SPEECH SIGNAL */
#if WMOPS
   move16();
#endif

#if DMEM
   awl = allocWord16(0, LPCO);
#endif
	awl[0] = plc->al[0];
	FOR (i=1;i<=LPCO; i++)
   {
      awl[i] = mult_r(STWAL[i],plc->al[i]);
#ifdef WMOPS
      move16();
#endif
   }
	apfilterQ1_Q0(awl, LPCO, xw+DFO, xw+DFO, FRSZ, plc->stwpml);
   W16copy(plc->stwpml, xw+DFO+FRSZ-LPCO, LPCO);

#if DMEM
   deallocWord16(awl, 0, LPCO);
#endif
#if DMEM
   xwd = allocWord16(0, LXD-1);
#endif
	/* PERFORM 8:1 DECIMATION, UPDATE DECIMATED WEIGHTED SPEECH BUFFER */
   decim(xw, xwd, plc);      
#if DMEM
   deallocWord16(xw, 0, DFO+FRSZ-1);
#endif
	/* DO COARSE PITCH EXTRACTION & PITCH REFINEMENT ONLY IN GOOD FRAME */
	IF (good_frame) 
   {
		/* GET THE COARSE VERSION OF PITCH PERIOD USING 8:1 DECIMATION */
		cpp = coarsepitch(xwd, plc->cpplast);
		plc->cpplast=cpp;
#if WMOPS
      move16();
#endif
   }
#if DMEM
   deallocWord16(xwd, 0, LXD-1);
#endif
   IF (good_frame)
   {
		/* REFINE PITCH PERIOD, FIND PITCH TAP & ANALYSIS WINDOW SIZE, ETC. */
		plc->pp=prfn(&plc->ptfe,&plc->cormax,&plc->energymax32,&plc->ppt,
			                &plc->wsz,&plc->scaled_flag,xq,cpp);
#ifdef WMOPS
      move16();
#endif
	}
    /* UPDATE PITCH PERIOD HISTORY BUFFER */
	FOR (i=PPHL-1;i>0;i--) 
   {
      plc->pph[i] = plc->pph[i-1];
#if WMOPS
      move16();
#endif
   }
	plc->pph[0] = plc->pp;
#if WMOPS
   move16();
#endif
    /* UPDATE SHORT-TERM SYNTHESIS FILTER MEMORY */
   W16copy(plc->stsyml, &xq[LXQ-LPCO], LPCO);

	/* SHIFT DECODED SPEECH BUFFER */
   W16copy(plc->xq,xq+FRSZ,XQOFF);
}

/*-----------------------------------------------------------------------------
 * Function: WB_PLC()
 *
 * Description: PLC function called in good frames
 *
 * Inputs:  *plc  - pointer to plc state memory
 *          *out  - pointer to output buffer
 *          *inbuf- pointer to the good frame input
 *
 * Outputs: *out  - potentially modified output buffer
 *---------------------------------------------------------------------------*/
void	WB_PLC(
struct 	WB_PLC_State *plc, 
Word16 	*out,
Word16 	*inbuf)
{
   Word16   *xq;
	Word32	a0;

	Word16	*pup, *pdown;
	Word16   i, length;
   Word16   *pring;
   Word16   outlen;

#if DMEM
   Word16   *out2;
#else
   short out2[FRSZ];
#endif


   xq = plc->xq;
   pring = &xq[LXQ];
   W16copy(xq+XQOFF,inbuf,FRSZ);

	IF (plc->cfecount != 0)	
   {
      plc->nfle = plc->cfecount; /* update Number of Frames in Last Erasure */
      plc->ngfae = 1; /* set Number of Good Frames After Erasure to 1 */
#if WMOPS
      move16();move16();
#endif
	} 
   ELSE 
   {
		 plc->ngfae++;   /* update Number of Good Frames After Erasure */
#if WMOPS
       move16();
#endif
       plc->ngfae = s_min(plc->ngfae, 9);
	}
	plc->cfecount=0;
#if WMOPS
   move16();
#endif

	IF (sub(plc->ngfae,1)==0) 
   {  /* IF THIS IS THE FIRST GOOD FRAME AFTER THE ERASURE */ 
      outlen = FRSZ;
#if WMOPS
      move16();
#endif
      IF (plc->lag!=0) /* -100 */
      {
#if DMEM
         out2 = allocWord16(0, FRSZ-1);
#endif
         /* refine the lag around the OLA window */
         plc->lag = refinelag( xq, plc->pp, inbuf, plc->lag);
#if WMOPS
         move16();
#endif
         outlen = add(FRSZ-MIN_UNSTBL, plc->lag);
         outlen = s_min(outlen, FRSZ);

         resample(inbuf+MIN_UNSTBL, out2+FRSZ-outlen, plc->lag);
         FOR (i=0;i<FRSZ-outlen+OLALG;i++)
         {
            xq[XQOFF+i] = round(L_shl(L_mult(plc->ptfe,xq[XQOFF+i-plc->pp]),1));
#ifdef WMOPS
            move16();
#endif
         }

         pup=olaug;
         pdown=oladg;

         FOR (i=0;i<OLALG;i++) 
         {
			   a0 = L_mult(xq[XQOFF+i],pup[i]);
			   a0 = L_mac(a0, pring[i], pdown[i]);
		  	   xq[XQOFF+i] = round(a0);
#ifdef WMOPS
            move16();
#endif
		   }

         FOR (i=FRSZ-outlen;i<FRSZ-outlen+OLALG;i++)
         {
			   a0 = L_mult(out2[i],*pup++);
			   a0 = L_mac(a0, xq[XQOFF+i], *pdown++);
		  	   xq[XQOFF+i] = round(a0);
#ifdef WMOPS
            move16();
#endif
		   }
         W16copy(&xq[XQOFF+i], &out2[i], FRSZ-i);

#if DMEM
         deallocWord16(out2, 0, FRSZ-1);
#endif
      }
      ELSE
      {
		  
        
		   IF (sub(plc->scaler,MAX_16)==0) 
         {  /* IF LAST FRAME IS BASICALLY UNVOICED (scaler=1), USE SHORT OVERLAP-ADD */ 
            length=SOLAL; 
            pup=olaup; 
            pdown=oladown; 
         }
		   ELSE 
         {				/* OTHERWISE, USE LONGER OVERLAP-ADD */
			   length=OLALG;
			   pup=olaug;
			   pdown=oladg;
		   }
   #if WMOPS
         move16();
   #endif
 
         /* PERFORM OVERLAP-ADD WITH RINGING OF CASCADED LT & ST SYNTHESIS FILTER */
         FOR (i=0;i<length;i++) 
         {
			   a0 = L_mult(xq[XQOFF+i],pup[i]);
			   a0 = L_mac(a0, pring[i], pdown[i]);
		  	   xq[XQOFF+i] = round(a0);
#ifdef WMOPS
            move16();
#endif
		   }
      }
      W16copy(out, xq+XQOFF, FRSZ); 
   }
	WB_PLC_common(plc,out,xq,1);
}

/*-----------------------------------------------------------------------------
 * Function: WB_PLC_erasure()
 *
 * Description: PLC function called in bad frames
 *
 * Inputs:  *plc  - pointer to plc state memory
 *          *out  - pointer to output buffer
 * Outputs: *qdb  - extrapolated samples beyond the output buffer required
 *                  for qmf memory, ringing, and rephasing
 *---------------------------------------------------------------------------*/
void    WB_PLC_erasure(
struct 	WB_PLC_State *plc,
Word16 	*out,
Word16   *qdb)
{
#if DMEM
   Word16   *ltring;
   Word16   *tmp;
   Word16   *ring;
#else
	Word16	ltring[OLALG];			/* Q1 */
	Word16	tmp[FRSZ+24+MAXOS];	/* Q0 */
   Word16   ring[OLAL+LPCO];
#endif
	Word32	a0;
   Word16   *xq;
	Word16	scalep, delta, gaw, upw;
	Word16	pp;
	Word16	n, i, n1, tmp16;
   Word16   *pring, *p_tmp;

	xq = plc->xq;
   pring = &xq[LXQ];

	plc->cfecount++;		/* update consecutive frame erasure counter */
	plc->ngfae=0;
	pp = plc->pp;
#if WMOPS
   move16();move16();move16();
#endif

	/* FIND PITCH PERIOD & SCALING FACTOR FOR WAVEFORM EXTRAPOLATION */
	IF (sub(plc->cfecount,1)==0) 
   { /* if it is the first erased frame */

      /* calcualte figure of merit to determine mixing ratio */ 
      plc->merit=merit(xq,plc->wsz,plc->cormax,
			        plc->energymax32,plc->scaled_flag);
      plc->ppinc=0;    /* Q7 pitch period increase initialized to 0 */
#if WMOPS
      move16();move16();
#endif

      /* CALCULATE AVERAGE PITCH PERIOD INCREASE AT THE FIRST ERASED FRAME */
		FOR (n=1;n<5;n++) 
      {
         n1 = sub(n,1);
			delta = sub(plc->pph[n-1],plc->pph[n]);	/* Q0 pitch period change */
#ifdef WMOPS
         test();
#endif
			IF ((delta > 0) && ( sub(i_mult(20,delta), plc->pph[n1])<0) ) 
         {
				plc->ppinc=round(L_shl(L_mult(delta, div_n[n1]), 6)); /* Q6 */
#if WMOPS
            move16();
#endif
				plc->ppinc = s_min(plc->ppinc,128);
				BREAK;
			}
         ELSE IF((delta < 0) && ( sub(i_mult(-20,delta),plc->pph[n1])<0)) 
         {
#ifdef WMOPS
            test();
#endif
            plc->ppinc=round(L_shl(L_mult(delta, div_n[n1]), 6)); /* Q7 */
#if WMOPS
            move16();
#endif
				plc->ppinc = s_max(plc->ppinc, -64);
				BREAK;
			}
		}
		plc->ppf = shl(pp,6); /* set Q6 version for later use */
#if WMOPS
      move16();
#endif
#if DMEM
      ltring = allocWord16(0, OLALG-1);
#endif

		/* CALCULATE AN APPROXIMATION OF LT SYNTHESIS FILTER RINGING. */
      /* FIRST, CALCULATE LPC PRESIDCTION RESIDUAL ON-THE-FLY */ 
		azfilterQ0_Q1(plc->al,LPCO,xq+XQOFF-pp,ltring,OLAL);

      /* scale LPC residual by pitch predictor tap */ 
      FOR (n=0;n<OLAL;n++) 
      {
         a0 = L_mult(ltring[n], plc->ppt);
         a0 = L_shl(a0, 6);
         ltring[n] = round(a0);       /* Q1 */
#if WMOPS
         move16();
#endif
		}
		/* FILTER LONG-TERM SYNTHSIS FILTER RINGING WITH LPC SYNTHESIS FILTER */
#if DMEM
      ring = allocWord16(0, OLAL+LPCO-1);
#endif
		apfilterQ1_Q0(plc->al,LPCO,ltring,ring+LPCO,OLAL,plc->stsyml);
      W16copy(pring, ring+LPCO, OLAL);

#if DMEM
      deallocWord16(ltring, 0, OLALG-1);
      deallocWord16(ring,   0, OLAL+LPCO-1);
#endif
	}

   IF (sub(plc->cfecount,2)==0) 
   {  /* if it is the 2nd consecutively erased frame, */
      /* change pitch period by adding ppinc */
		plc->ppf = add(plc->ppf, plc->ppinc);	
#if WMOPS
      move16();
#endif
		pp=shr(add(plc->ppf,32),6); /* round off to nearest integer */
		pp = s_min(pp,MAXPP);
		pp = s_max(pp,MINPP);
		plc->ppf = pp;
#if WMOPS
      move16();
#endif
	} 
#ifdef WMOPS
   test();
#endif
   if ((sub(plc->cfecount,1)!=0)&&(sub(plc->cfecount,2)!=0)) {
		pp = plc->ppf;
#if WMOPS
      move16();
#endif
	}
	plc->pp = pp;
#if WMOPS
   move16();
#endif
    /* IF FIGURE OF MERIT > LOW LIMIT, DO PERIODIC WAVEFORM EXTRAPOLATION */ 
	IF (sub(plc->merit,256*MLO)>0) 
   {
		/* EXTRAPOLATE BY OLAL SAMPLES, OVERLAP-ADD with RINGING */
		delta = 1638;
		upw = 0;
#if WMOPS
      move16();move16();
#endif
      /* first-phase extrapolation */
		FOR (i=0;i<OLAL;i++) 
      { 
			tmp16 = round(L_shl(L_mult(plc->ptfe,xq[XQOFF+i-pp]),1)); 
			upw = add(upw, delta);
			a0 = L_mult(tmp16,upw);
			a0 = L_mac(a0,pring[i],sub(MAX_16,upw));
			xq[XQOFF+i] = round(a0);
#ifdef WMOPS
         move16();
#endif
		}

		/* SECOND-PHASE EXTRAPOLATION OF DECODER OUTPUT SPEECH SIGNAL */
		FOR (i=OLAL;i<FRSZ+24+MAXOS;i++)    
      {
		    xq[XQOFF+i] = round(L_shl(L_mult(plc->ptfe,xq[XQOFF+i-pp]),1));
#ifdef WMOPS
          move16();
#endif
      }

	}

	/* IF FIGURE OF MERIT < HIGH LIMIT, FEED WHITE NOISE THROUGH LPC FILTER,
       AND MIX THE RESULT WITH PERIODICALLY EXTRAPOLATED WAVEFORM IF NEEDED */
   IF (sub(plc->merit, 256*MHI)<=0) 
   {   
		/* GENERATE ONE FRAME OF WHITE GAUSSIAN RANDOM NOISE WITH std = avm */
      n=plc->cfecount;
#if WMOPS
      move16();
#endif
#if DMEM
      tmp = allocWord16(0, FRSZ+24+MAXOS-1);
#endif
      FOR (i=0;i<FRSZ+24+MAXOS;i++,) 
      {
         if (sub(n,126)>0) n=sub(n,127);  /* modulo indexing */ 
         tmp[i] = mult_r(plc->avm,wn[s_and(n,127)]);	/* Q2*Q13 -> Q0 */
         n=add(n,plc->cfecount);
#if WMOPS
         move16();
#endif
      }

		/* FILTER THE WHITE NOISE WITH LPC SYNTHESIS FILTER */
		apfilterQ0_Q0(plc->al,LPCO,tmp,tmp,FRSZ+24+MAXOS,plc->stsyml);

		/* CALCULATE THE SCALING FACTORS FOR THE MIXTURE */
      IF (sub(plc->merit, i_mult(256,MLO))>0 ) 
      {
         /* scaling factor for random noise component */
         plc->scaler=shl(sub(MHI*256,plc->merit),4);	/* Q15 */
#ifdef WMOPS
         move16();
#endif
		   scalep=sub(MAX_16,plc->scaler); /* scaling for periodic component */
		    /* MIX THE TWO COMPONENTS */
         p_tmp = tmp;
         FOR (i=XQOFF;i<LXQ+24+MAXOS;i++) 
         {
            a0 = L_mult(scalep, xq[i]);
		      a0 = L_mac(a0, plc->scaler, *p_tmp++);
		      xq[i] = round(a0);
#if WMOPS
            move16();
#endif
          }
		} 
      ELSE 
      {
          plc->scaler=MAX_16;	/* scaling factor for noise component */	
#if WMOPS
          move16();
#endif
          W16copy(&xq[XQOFF], tmp, LXQ+24+MAXOS-XQOFF+1);
		}
#if DMEM
      deallocWord16(tmp, 0, FRSZ+24+MAXOS-1);
#endif
	} 
   ELSE 
   {
        plc->scaler=0;
#if WMOPS
        move16();
#endif
	}
	/* IF MORE THAN GATTST FRAMES INTO ERASURE, APPLY GAIN ATTENUATION WINDOW */

   IF (sub(plc->cfecount,GATTST)>0)  
   {
      IF (sub(plc->cfecount,GATTEND)<=0) 
      {
			delta = gawd[sub(plc->cfecount, (GATTST+1))];
			gaw = MAX_16;
#if WMOPS
			move16();move16();
#endif
         FOR (i=XQOFF; i<XQOFF+FRSZ; i++) 
         {
            xq[i] = mult(xq[i], gaw);
            gaw = add(gaw, delta);
#ifdef WMOPS
            move16();
#endif
			}
         IF(sub(plc->cfecount, GATTEND) < 0)
         {
            FOR (i=XQOFF+FRSZ; i<XQOFF+FRSZ+24+MAXOS; i++) 
            {
               xq[i] = mult(xq[i], gaw);
               gaw = add(gaw, delta);
#ifdef WMOPS
               move16();
#endif
            }
         }
         ELSE
         {
            W16zero(&xq[XQOFF+FRSZ], 24+MAXOS);
         }
      } 
      ELSE /* IF MORE THAN GATTEND FRAMES INTO ERASURE, MUTE OUTPUT SIGNAL */
      {
         W16zero(&xq[XQOFF],FRSZ+24+MAXOS);
      }
   }
	WB_PLC_common(plc,out,xq,0);
   W16copy(qdb, &xq[XQOFF+FRSZ], 22+MAXOS);
}


/*-----------------------------------------------------------------------------
 * Function: hsbupd()
 *
 * Description: Updates the high-band ADPCM decoder state memory during
 *              lost packets.
 *
 * Inputs:  *plc     - plc state memory
 *          *s       - G.722 state memory
 *          *out     - high-band PLC component
 *          Nsamples - frame length
 *
 * Outputs: *s       - G.722 state memory
 *---------------------------------------------------------------------------*/
void hsbupd(struct WB_PLC_State *plc, g722_state *s, short *out, short Nsamples)
{
	short i;
	Word16 phhp, rhhp, phl, rhl;
	Word32 a0;
	Word16 ph_prop, ph_cnst, ph_postn;
	Word16 dh, sh;

#if WMOPS
	test();
#endif
	IF(sub(plc->cfecount, GATTEND) >= 0 || sub(plc->hb_reset,1) == 0)
   {
      reset_hsbdec(s, plc);
	}
	ELSE
   {
      ph_cnst  = 0;
      phhp = plc->phhp_m1;
      phl = plc->ph_m1;
      ph_postn = plc->ph_postn;
      rhhp = plc->rhhp_m1;
      rhl = plc->rh_m1;
	   sh = s->sh;
#if WMOPS
      move16();move16();move16();move16();move16();move16();move16();
#endif

		FOR (i=0;i<Nsamples;i++)
      {
			dh = sub (out[i], sh);
			s->ph[0] = add (dh, s->szh);
#ifdef WMOPS
         move16();
#endif
			if(s->ph[0] > 0)
         {
				ph_postn = add(ph_postn, 1);
         }
			if(s->ph[0] < 0)
         {
				ph_postn = sub(ph_postn, 1);
         }
			if(sub(s->ph[0],s->ph[1]) == 0)
				ph_cnst  = add(ph_cnst, 1);

			/* update memory of DC removal filter on ph */
			a0 = L_mult(AHP,phhp);
			a0 = L_msu(a0,AHP,phl);
			phl = shl(s->ph[0],4);
         a0 = L_mac(a0,AHP,phl);
			phhp = round(a0);

			/* update memory of DC removal filter on rh */
			a0 = L_mult(AHP,rhhp);
			a0 = L_msu(a0,AHP,rhl);
			rhl = shl(out[i],4);
			a0 = L_mac(a0,AHP,rhl);
			rhhp = round(a0);

			s->rh[0] = shl(out[i],1);
			s->dh[0] = shl(dh,1);

#ifdef WMOPS
			move16();move16();
#endif

         /* update ADPCM predictor coefficients and filter memory */
			sh = plc_adaptive_prediction(s->dh, s->bh, s->ah, s->ph, 15360, s->rh, &(s->szh));
		}
      plc->phhp_m1 = phhp;
      plc->ph_m1 = phl;
      plc->ph_postn = ph_postn;
      plc->rhhp_m1 = rhhp;
      plc->rh_m1 = rhl;
	   s->sh = sh;
#ifdef WMOPS
      move16();move16();move16();move16();move16();move16();
#endif

		IF(sub(plc->cfecount,2) > 0)
      { /* current plc->pl_postn is based on */
	                                  /* cfecount frames                   */

			/* Only enters here for 3 or more lost frames */

			/* Normalize property of ph over lost frames */
			ph_prop = mult_r(plc->ph_postn,inv_frm_size[plc->cfecount-3]);
#ifdef WMOPS
         test();   
#endif
			IF(sub(abs_s(ph_prop),36) > 0 || sub(ph_cnst,40) > 0)
         {
				plc->hb_reset = 1;
#if WMOPS
				move16();
#endif
				/* reset HB ADPCM decoder adaptively */
            reset_hsbdec(s, plc);
			}
		}
	}
}


/*-----------------------------------------------------------------------------
 * Function: lsbupd()
 *
 * Description: Updates the low-band ADPCM decoder state memory during
 *              lost packets.
 *
 * Inputs:  *plc     - plc state memory
 *          *s       - G.722 state memory
 *          *out     - low-band PLC component
 *          Nsamples - frame length
 *
 * Outputs: *s       - G.722 state memory
 *---------------------------------------------------------------------------*/
void lsbupd (struct WB_PLC_State *plc, g722_state *s, short *out, short Nsamples)
{
   Word16 i, nbpl;
   Word16 pl_prop, pl_cnst;
   Word16 detl;
   Word16 pl_postn;
   Word16 dlt, sl;

#ifdef WMOPS
   test();
#endif
   IF(sub(plc->cfecount, GATTEND) >= 0 || sub(plc->lb_reset,1) == 0)
   {
      plc->lb_reset = 1;
#if WMOPS
      move16();
#endif

      /* Only reset the nbpl and pole section tracking if output has been muted */
      if(sub(plc->cfecount,GATTEND) >= 0)
      {
         plc->psml_mean = 1024;
         plc->nbpl_mean1 = 0;
         plc->nbpl_mean2 = 0;
         plc->nbpl_trck  = 0;
         plc->nbpl_chng  = 0;
#if WMOPS
         move16();move16();move16();move16();move16();
#endif
      }

      /* Reset LB ADPCM decoder for extended losses */
      reset_lsbdec(s);
      plc->lb_reset = 1;
#if WMOPS
      move16();
#endif
   }
   ELSE
   {
      pl_cnst  = 0;
      nbpl = s->nbl;
      detl = s->detl;
      pl_postn = plc->pl_postn;
      sl = s->sl;
#if WMOPS
      move16();move16();move16();move16();move16();
#endif

      FOR (i=0;i<Nsamples;i++)
      {
         dlt = sub (out[i], sl);

		  /* update adaptive scaling factor */
         nbpl = quantl_toupdatescaling_logscl (dlt, detl, nbpl);
         detl = scalel (nbpl);
         s->plt[0] = add (dlt, s->szl);
#ifdef WMOPS
         move16();
#endif
         if(s->plt[0] > 0)
            pl_postn = add(pl_postn, 1);
         if(s->plt[0] < 0)
            pl_postn = sub(pl_postn, 1);    
         if(sub(s->plt[0],s->plt[1]) == 0)
            pl_cnst  = add(pl_cnst, 1);

         s->rlt[0] = shl(out[i],1);
         s->dlt[0] = shl(dlt,1);
#if WMOPS
         move16();move16();
#endif

         /* update ADPCM predictor coefficients and filter memory */
         sl = plc_adaptive_prediction(s->dlt, s->bl, s->al, s->plt, 15360, s->rlt, &(s->szl));
      }
      s->nbl  = nbpl;
      s->detl = detl;
      plc->pl_postn = pl_postn;
      s->sl = sl;
#if WMOPS
      move16();move16();move16();move16();
#endif
      IF(sub(plc->cfecount,2) > 0)
      { /* current plc->pl_postn is based on */
	     /* cfecount frames                   */

		  /* Only enters here for 3 or more lost frames */

		  /* Normalize property of pl over lost frames */
         pl_prop = mult_r(plc->pl_postn,inv_frm_size[plc->cfecount-3]);
#ifdef WMOPS
         test();
#endif
         IF(sub(abs_s(pl_prop),36) > 0 || sub(pl_cnst,40) > 0)
         {
            plc->lb_reset = 1;
#if WMOPS
            move16();
#endif
            /* reset HB ADPCM decoder adaptively */
            reset_lsbdec (s);
         }
      }
   }
}

/*-----------------------------------------------------------------------------
 * Function: dltdec()
 *
 * Description: partial decoding of G722 "dlt" signal.
 *
 * Inputs:  *code  - pointer to channel indices
 *          detl   - detl memory from previous sample
 *          nbl    - nbl  memory from previous sample
 *          Nsamples - number of samples of dlt to generate
 * Outputs: *out  - decoded dlt signal
 *---------------------------------------------------------------------------*/
void dltdec(short *code, short detl, short nbl, short *out, short Nsamples)
{
   short il;
   int i;

   FOR (i = 0; i < Nsamples; i++)
   {
      il     = s_and(code[i], 0x3F);	/* 6 bits of low SB */
      out[i] = invqal (il, detl);
      nbl    = logscl (il, nbl);
      detl   = scalel (nbl);
#ifdef WMOPS
      move16();
#endif
   }
}

/*-----------------------------------------------------------------------------
 * Function: filtdlt()
 *
 * Description: filtering of dlt signal by a fixed pole-zero filter
 *
 * Inputs:  *in      - pointer to in buffer containing dlt signal
 *          *s       - pointer to g722 state
 *          Nsamples - number of samples to filter
 * Outputs: *out     - output signal
 *---------------------------------------------------------------------------*/
void filtdlt(short *in, g722_state *s, short *out, short Nsamples)
{
   int i;
   long acc;

   out[0]  = shr(s->rlt[0],1);
   out[-1] = shr(s->rlt[1],1);
#ifdef WMOPS
   move16();move16();
#endif

   FOR (i=0;i<Nsamples-1; i++)
   {
      /* zero section */
      acc = L_mult(in[i-1], s->bl[1]);
      acc = L_mac(acc, in[i-2], s->bl[2]);
      acc = L_mac(acc, in[i-3], s->bl[3]);
      acc = L_mac(acc, in[i-4], s->bl[4]);
      acc = L_mac(acc, in[i-5], s->bl[5]);
      acc = L_mac(acc, in[i-6], s->bl[6]);

      /* pole section */
      acc  = L_mac(acc,  s->al[1], out[i]);
      acc  = L_mac(acc, s->al[2], out[i-1]);
      out[i+1] = add(round(L_shl(acc,1)), in[i+1]);
#ifdef WMOPS
      move16();
#endif
   }
   out[0] = add(in[0], s->sl);
#ifdef WMOPS
   move16();
#endif
}


/*-----------------------------------------------------------------------------
 * Function: plc_adaptive_prediction()
 *
 * Description: Updates ADPCM predictor coefficients and filter memory - sample-
                based.
 *
 * Inputs:  *d  - difference signal sample
 *          *b  - zero-section coefficients
 *          *a  - pole-section coefficients
 *          *p  - partially reconstructed signal sample
 *          safetythres - pole section safety threshold
 *          *r  - reconstructed signal sample
 *
 * Outputs: *b  - zero-section coefficients
 *          *a  - pole-section coefficients
 *          *sz - zero-section predicted signal sample
 *          s   - predicted signal sample
 *---------------------------------------------------------------------------*/
Word16 plc_adaptive_prediction(Word16 *d, Word16 *b, Word16 *a, Word16 *p, 
							   Word16 safetythres, Word16 *r, Word16 *sz)
{
	Word16 wd1, wd2, s1, s2, a1, a2, r_sz, sp, s, i;

	if(d[0] == 0){
		wd1 = 0;
#ifdef WMOPS
		move16();
#endif
	}
	if(d[0] != 0){
		wd1 = 128;
#ifdef WMOPS
		move16();
#endif
	}

   /* update zero-section coefficients and shift difference signal memory */
	FOR (i = 6; i > 0; i--){
		wd2 = mult (b[i], 32640);
		s1 = s_xor(d[0],d[i]);
		if(s1 >= 0)
			wd2 = add (wd2, wd1);
		if(s1 < 0)
			wd2 = sub (wd2, wd1);

		b[i] = wd2;
		d[i] = d[i-1];

#ifdef WMOPS
		move16();
		move16();
#endif
	}

   /* update 2nd pole-section coefficient */
	wd1 = shl (a[1], 2);
	s1 = s_xor(p[0], p[1]);
	if(s1 >= 0)
		wd1 = sub (0, wd1);
	wd1 = shr (wd1, 7);

	s2 = s_xor(p[0], p[2]);
	if(s2 >= 0)
		wd1 = add (wd1, 128);
	if(s2 < 0)
		wd1 = sub (wd1, 128);

	wd2 = mult(a[2], 32512);
	a2 = add(wd2, wd1);

	a2 = s_min(a2, 12288);
	a2 = s_max(a2, -12288);

	a[2] = a2;
#ifdef WMOPS
	move16();
#endif

   /* update 1st pole-section coefficient */
	wd1 = mult (a[1], 32640);
	if(s1 >= 0)
		a1 = add(wd1, 192);
	if(s1 < 0)
		a1 = sub(wd1, 192);
	wd2 = sub (safetythres, a[2]);
	a1 = s_min(a1, wd2);

	if(add(a1, wd2) < 0)
		a1 = negate (wd2);

	/* shift the partial reconstructed signal memory */
	p[2] = p[1];
	p[1] = p[0];
	a[1] = a1;
#ifdef WMOPS
	move16();
	move16();
	move16();
#endif

   /* zero-section prediction update */
	r_sz = mult(d[6], b[6]);
	wd1  = mult(d[5], b[5]);
	r_sz = add(r_sz, wd1);
	wd1  = mult(d[4], b[4]);
	r_sz = add(r_sz, wd1);
	wd1  = mult(d[3], b[3]);
	r_sz = add(r_sz, wd1);
	wd1  = mult(d[2], b[2]);
	r_sz = add(r_sz, wd1);
	wd1  = mult(d[1], b[1]);
	r_sz = add(r_sz, wd1);

	/* shift the reconstructed signal memory */
	r[2] = r[1];		
	r[1] = r[0];		
#ifdef WMOPS
	move16();
	move16();
#endif

   /* pole-section prediction update */
   wd1 = mult(a[1], r[1]);
	wd2 = mult(a[2], r[2]);
	sp  = add(wd1, wd2);
	*sz = r_sz;

   /* prediction update */
	s   = add(*sz, sp);
#ifdef WMOPS
	move16();
#endif

	return s;
}


/*-----------------------------------------------------------------------------
 * Function: plc_lsbdec()
 *
 * Description: PLC low-band decoding - sample-based.
 *
 * Inputs:  ilr  - index
 *          mode - G.722 mode
 *          rs   - reset flag
 *          *s   - G.722 state
 *
 * Outputs: yl   - low-band reconstructed signal sample
 *          *s   - G.722 state
 *---------------------------------------------------------------------------*/
Word16 plc_lsbdec (Word16 ilr, Word16 mode, Word16 rs, g722_state *s, Word16 safetythres)
{
  Word16 dl, rl, nbpl, yl;
  Word16 dlt;

  dl = invqbl (ilr, s->detl, mode);
  rl = add (s->sl, dl);
  yl = limit (rl);

  dlt = invqal (ilr, s->detl); /* truncated quantized diff signal */
  nbpl = logscl (ilr, s->nbl); /* delta L (n) Q11 (eq. 3-13) */
  s->nbl = nbpl;
  s->detl = scalel (nbpl); /* convert to linear 3-17 */
  s->plt[0] = add (dlt, s->szl); /* 3-27 */
  s->rlt[0] = shl(add (s->sl, dlt),1); /* 3-25 */
  s->dlt[0] = shl(dlt, 1);
#ifdef WMOPS
  move16();
  move16();
  move16();
  move16();
  move16();
#endif

  /* update ADPCM predictor coefficients and filter memory */
  s->sl = plc_adaptive_prediction(s->dlt, s->bl, s->al, s->plt, safetythres, s->rlt, &(s->szl));

#ifdef WMOPS 
  move16();
#endif

  return (yl);

}


/*-----------------------------------------------------------------------------
 * Function: plc_hsbdec()
 *
 * Description: PLC high-band decoding - sample-based.
 *
 * Inputs:  ih   - index
 *          *s   - G.722 state
 *          *plc - plc state
 *          *pNBPHlpfilter - pointer to lp filter function
 *          *pDCremoval - pointer to DC removal function
 *          inv_frames_int - 
 *          inv_frames_frc - 
 *          sample - sample number
 *          rs   - reset flag (not used)
 *
 * Outputs: yh   - high-band reconstructed signal sample
 *          *s   - G.722 state
 *---------------------------------------------------------------------------*/
Word16 plc_hsbdec (Word16 ih, Word16 rs, g722_state *s, struct WB_PLC_State *plc,
                   Word16 (*pNBPHlpfilter)( struct WB_PLC_State *plc, Word16 inv_frames_int, 
                     Word16 inv_frames_frc, Word16 nbph, Word16 sample), 
                     Word16 (*pDCremoval)(Word16 rh0, Word16 *rhhp, Word16 *rhl),
                     Word16 inv_frame_int, Word16 inv_frame_frc, Word16 sample)
{
   Word16 nbph, yh;
   Word16 ph0, rh0;
   Word16 dh,rh;

   dh = invqah (ih, s->deth);
   nbph = logsch (ih, s->nbh);
   s->nbh = nbph;
#ifdef WMOPS
   move16();
#endif

  /* Adaptive LP filter on nbph */
   nbph = pNBPHlpfilter( plc, inv_frame_int, inv_frame_frc, nbph, sample);
   s->deth = scaleh (nbph);
#ifdef WMOPS
   move16();
#endif

   ph0 = add (dh, s->szh);

   /* DC removal filter on ph */
   s->ph[0] = pDCremoval(ph0, &(plc->phhp_m1), &(plc->ph_m1));
#ifdef WMOPS
   move16();
#endif

   rh0 = add (s->sh, dh);
  /* DC removal filter on rh */
   rh = pDCremoval(rh0, &(plc->rhhp_m1), &(plc->rh_m1));

   s->dh[0] = shl(dh, 1);
   s->rh[0] = shl(rh, 1);
#ifdef WMOPS
   move16();move16();
#endif

  /* update ADPCM predictor coefficients and filter memory */
   s->sh = plc_adaptive_prediction(s->dh, s->bh, s->ah, s->ph, 15360, s->rh, &(s->szh));

#ifdef WMOPS
   move16();
#endif

   yh = limit (rh);

   return (yh);
}


/*-----------------------------------------------------------------------------
 * Function: quantl_toupdatescaling_logscl()
 *
 * Description: update low-band adaptive log scale factor - sample-based.
 *
 * Inputs:  el   - error signal sample
 *          detl - current linear scaling factor
 *          nbl  - previous log scaling factor
 *
 * Outputs: nbpl - updated log scaling factor
 *---------------------------------------------------------------------------*/
Word16 quantl_toupdatescaling_logscl (Word16 el, Word16 detl, Word16 nbl)
{
   Word16 sil, wd;
   Word16 *p_id, *p_q4, q4_scl;
   Word16 nbpl;

   /* quantize input to the required 8 levels for log scale factor */
   sil = shr (el, 15);

   wd = s_and(el, MAX_16);
   if(sil == 0)
   {
      wd = el;
#ifdef WMOPS
      move16();
#endif
   }
   if (sil!=0)
      wd=sub(MAX_16, wd);

   p_q4 = q4+7;
   p_id = wlil4rilil+8;

   q4_scl = mult(*p_q4--, detl);
   if(sub(wd,q4_scl)<0)
	   p_id--;
   q4_scl = mult(*p_q4--, detl);
   if(sub(wd,q4_scl)<0)
	   p_id--;
   q4_scl = mult(*p_q4--, detl);
   if(sub(wd,q4_scl)<0)
	   p_id--;
   q4_scl = mult(*p_q4--, detl);
   if(sub(wd,q4_scl)<0)
	   p_id--;
   q4_scl = mult(*p_q4--, detl);
   if(sub(wd,q4_scl)<0)
	   p_id--;
   q4_scl = mult(*p_q4--, detl);
   if(sub(wd,q4_scl)<0)
	   p_id--;
   q4_scl = mult(*p_q4--, detl);
   if(sub(wd,q4_scl)<0)
	   p_id--;
   if(wd<0)
	   p_id--;

   /* update low-band log scale factor */
   wd = mult (nbl, 32512);
   nbpl = add (wd, *p_id);

   if(nbpl < 0)
   {
	   nbpl = 0;
#ifdef WMOPS
	   move16();
#endif
   }
   nbpl = s_min(nbpl, 18432);

   return (nbpl);
}

#endif /* G722DEMO */

/*-----------------------------------------------------------------------------
 * Function: Reset_WB_PLC()
 *
 * Description: reset the plc state variables
 *
 * Inputs:  *plc  - pointer to plc state memory
 *
 * Outputs: *plc  - values reset
 *---------------------------------------------------------------------------*/
void Reset_WB_PLC(struct WB_PLC_State *plc)
{
	int	i;

   W16zero((Word16 *) plc, sizeof(struct WB_PLC_State)/2);
	plc->al[0]=4096;
	plc->alast[0]=4096;
	plc->xwd_exp=31;
	plc->cpplast=i_mult(12, cpp_scale);
	plc->pp=50;
	plc->ngfae=9;
   plc->wsz=1;
#if WMOPS
   move16();move16();move16();move32();move16();move16();move16();
#endif
	FOR (i=0;i<PPHL;i++) 
   {
      plc->pph[i] = plc->pp;
#if WMOPS
      move16();
#endif
   }

   /* State variables to interface with G.722 */

	/* Low-band */
	plc->psml_mean = 1024; /* G.722 standard value - 2^(-4) in Q14. */
	plc->nbpl_mean1 = 0;
	plc->nbpl_mean2 = 0;
	plc->nbpl_trck  = 0;
	plc->nbpl_chng  = 0;
	plc->pl_postn = 0;
	plc->lb_reset = 0;

	/* High-band */
	plc->nbph_mean = 0;
	plc->nbph_trck = 0;
	plc->nbph_chng = 0;
	plc->nbh_mode = 0;
	plc->hp_flag = 0;
	plc->nbph_lp = 0;

	plc->rhhp_m1 = 0;
	plc->rh_m1 = 0;
	plc->phhp_m1 = 0;
	plc->ph_m1 = 0;
	plc->ph_postn = 0;
	plc->hb_reset = 0;

	plc->sb_sample = 0;

#if WMOPS
      move16();move16();move16();move16();move16();
      move16();move16();move16();move16();move16();
      move16();move16();move16();move16();move16();
      move16();move16();move16();move16();move16();
	  move16();move16();move16();move16();move16();
#endif
}

#ifndef G722ENC
/*-----------------------------------------------------------------------------
 * Function: reset_lsbdec()
 *
 * Description: Reset of G.722 low-band decoder.
 *
 * Inputs:  *s  - G.722 state memory
 *
 * Outputs: *s  - G.722 state memory
 *---------------------------------------------------------------------------*/
void reset_lsbdec (g722_state *s)
{
   s->detl = 32;
   s->sl = s->spl = s->szl = s->nbl = 0;
   s->al[1] = s->al[2] = 0;
   s->bl[1] = s->bl[2] = s->bl[3] = s->bl[4] = s->bl[5] = s->bl[6] = 0;
   s->dlt[0] = s->dlt[1] = s->dlt[2] = s->dlt[3] = s->dlt[4] = s->dlt[5] = s->dlt[6] = 0;
   s->plt[0] = s->plt[1] = s->plt[2] = 0;
   s->rlt[0] = s->rlt[1] = s->rlt[2] = 0;


#ifdef WMOPS
    move16();
    move16();
    move16();
    move16();
    move16();
    move16();
    move16();
    move16();
    move16();
    move16();
    move16();
    move16();
    move16();
    move16();
    move16();
    move16();
    move16();
    move16();
    move16();
    move16();
    move16();
    move16();
    move16();
    move16();
    move16();
    move16();
#endif

}


/*-----------------------------------------------------------------------------
 * Function: hsbdec_resetg722()
 *
 * Description: Reset of G.722 high-band decoder.
 *
 * Inputs:  *s  - G.722 state memory
 *
 * Outputs: *s  - G.722 state memory
 *---------------------------------------------------------------------------*/
void hsbdec_resetg722(g722_state *s)
{
    s->deth = 8;
    s->sh = s->sph = s->szh = s->nbh = 0;
    s->ah[1] = s->ah[2] = 0;
    s->bh[1] = s->bh[2] = s->bh[3] = s->bh[4] = s->bh[5] = s->bh[6] = 0;
    s->dh[0] = s->dh[1] = s->dh[2] = s->dh[3] = s->dh[4] = s->dh[5] = s->dh[6] = 0;
    s->ph[0] = s->ph[1] = s->ph[2] = 0;
    s->rh[0] = s->rh[1] = s->rh[2] = 0;
#ifdef WMOPS
    move16();
    move16();
    move16();
    move16();
    move16();
    move16();
    move16();
    move16();
    move16();
    move16();
    move16();
    move16();
    move16();
    move16();
    move16();
    move16();
    move16();
    move16();
    move16();
    move16();
    move16();
    move16();
    move16();
    move16();
    move16();
    move16();
#endif
}


/*-----------------------------------------------------------------------------
 * Function: reset_hsbdec()
 *
 * Description: Reset of G.722 high-band decoder and related PLC variables.
 *
 * Inputs:  *s   - G.722 state memory
 *          *plc - PLC state memory
 *
 * Outputs: *s   - G.722 state memory
 *          *plc - PLC state memory
 *---------------------------------------------------------------------------*/
void reset_hsbdec (g722_state *s, struct WB_PLC_State *plc)
{

   hsbdec_resetg722(s);

   plc->phhp_m1 = 0;
   plc->ph_m1   = 0;
   plc->rhhp_m1 = 0;
   plc->rh_m1   = 0;	
   plc->nbph_mean = 0;
   plc->nbph_trck = 0;
   plc->nbph_chng = 0;
   plc->nbh_mode = 0;
   plc->hb_reset = 1;
#ifdef WMOPS
   move16();
   move16();
   move16();
   move16();
   move16();
   move16();
   move16();
   move16();
   move16();
#endif
}

/*-----------------------------------------------------------------------------
 * Function: NBPHlpfilter()
 *
 * Description: LP pass filter on nbph - sample-based.
 *
 * Inputs:  *plc - plc state memory
 *          inv_frames_int - 
 *          inv_frames_frc - 
 *          nbph - high-band log scale factor
 *          sample - sample number
 *
 * Outputs: nbph - high-band log scale factor
 *---------------------------------------------------------------------------*/
Word16 NBPHlpfilter( struct WB_PLC_State *plc, Word16 inv_frames_int, 
                     Word16 inv_frames_frc, Word16 nbph, Word16 sample)
{
   Word16 smpl, a_lp_lin, a_lp, a_lp_m1;
   Word32 a0;

   smpl = add(NGFAEOFFSET_P1[plc->ngfae],sample);	/* Q0  */
   a0 = L_mult(smpl, inv_frames_int);			/* Q16 */
   a0 = L_shl(a0, 15);										/* Q31 */
   a0 = L_mac(a0, smpl, inv_frames_frc);			/* Q31 */
   a_lp_lin = round(a0);										/* Q15 */
   a_lp = round(L_mult(a_lp_lin, a_lp_lin));					/* Q15 */
   a_lp_m1 = add(-32768,a_lp);								/* Q15 */
   a0 = L_mult(a_lp, nbph);
   a0 = L_msu(a0, a_lp_m1, plc->nbph_lp);
   nbph = round(a0);
   plc->nbph_lp = nbph;
#ifdef WMOPS
   move16();
#endif
   return(nbph);
}


/*-----------------------------------------------------------------------------
 * Function: NBPHnofilter()
 *
 * Description: No LP pass filter on nbph - sample-based.
 *
 * Inputs:  *plc - plc state memory
 *          inv_frames_int - 
 *          inv_frames_frc - 
 *          nbph - high-band log scale factor
 *          sample - sample number
 *
 * Outputs: nbph - high-band log scale factor
 *---------------------------------------------------------------------------*/
Word16 NBPHnofilter( struct WB_PLC_State *plc, Word16 inv_frames_int, 
                     Word16 inv_frames_frc, Word16 nbph, Word16 sample)
{
   return(nbph);
}

/*-----------------------------------------------------------------------------
 * Function: DCremoval()
 *
 * Description: DC removal filter (HP filter) - sample based.
 *
 * Inputs:  x0  - signal sample
 *          xhp - previous high-pass filtered signal sample
 *          x1  - previous signal sample
 *
 * Outputs: xhp - high-pass filtered signal sample
 *          xhp - high-pass filtered signal sample
 *          x1  - signal sample
 *---------------------------------------------------------------------------*/
Word16 DCremoval(Word16 x0, Word16 *xhp, Word16 *x1)
{
   Word32 a0;
   
   a0 = L_mult(AHP, *xhp);
   a0 = L_msu(a0,AHP, *x1);
   *x1 = shl(x0,4);
   a0 = L_mac(a0,AHP, *x1);
   *xhp = round(a0);
#if WMOPS
   move16(); move16();
#endif
   return(shr(add(*xhp,8),4));
}


/*-----------------------------------------------------------------------------
 * Function: DCremovalMemUpdate()
 *
 * Description: DC removal filter (HP filter).  Only memory update - sample based.
 *
 * Inputs:  x0  - signal sample
 *          xhp - previous high-pass filtered signal sample
 *          x1  - previous signal sample
 *
 * Outputs: x0  - Non-high-pass filtered signal sample
 *          xhp - high-pass filtered signal sample
 *          x1  - signal sample
 *---------------------------------------------------------------------------*/
Word16 DCremovalMemUpdate(Word16 x0, Word16 *xhp, Word16 *x1)
{
   Word32 a0;
   
   a0 = L_mult(AHP,*xhp);
   a0 = L_msu(a0,AHP,*x1);
   *x1 = shl(x0,4);
   a0 = L_mac(a0,AHP,*x1);
   *xhp = round(a0);
#if WMOPS
   move16(); move16();
#endif
   return(x0);
}
#endif /* G722ENC */

