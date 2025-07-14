/* ITU G.722 3rd Edition (2012-09) */

/*
  ============================================================================
   File: G722.H                                  v3.0 beta - 23/Aug/2006
  ============================================================================

                            UGST/ITU-T G722 MODULE

                          GLOBAL FUNCTION PROTOTYPES

  History:
14.Mar.95  v1.0       Released for use ITU-T UGST software package Tool
                      based on the CNET's 07/01/90 version 2.00
01.Jul.95  v2.0       Changed function declarations to work with many compilers;
                      reformated <simao@ctd.comsat.com>
23.Aug.06  v3.0 beta  Updated with STL2005 v2.1 basic operators and G.729.1 methodology
                      <{balazs.kovesi,stephane.ragot}@orange-ft.com>
  ============================================================================
*/
/* ITU-T G.722 PLC Candidate                                                     */
/* Version:       1.0                                                            */
/* Revision Date: Nov.02, 2006                                                   */

/*
  ITU-T G.722 PLC    ANSI-C Source Code

  This software has been developed by Broadcom Corporation. 
  
  Copyright (c)  Broadcom Corporation 2006.  All rights reserved. 

  COPYRIGHT : This file is the property of Broadcom Corporation.  It cannot 
  be copied, used, distributed or modified without obtaining authorization 
  from Broadcom Corporation.  If such authorization is provided, any modified 
  version of the software must contain this header.

  WARRANTIES : This software is made available by  Broadcom Corporation in the 
  hope that it will be useful, but without any warranty, including but not 
  limited to any warranty of non-infringement of any third party intellectual 
  property rights.  Broadcom Corporation is not liable for any direct or 
  indirect consequence  or damages related to the use of the provided software, 
  whether or not foreseeable .
*/

#ifndef G722_H
#define G722_H 200

/* DEFINITION FOR SMART PROTOTYPES */
#ifndef ARGS
#if (defined(__STDC__) || defined(VMS) || defined(__DECC)  || defined(MSDOS) || defined(__MSDOS__)) || defined (__CYGWIN__) || defined (_MSC_VER)
#define ARGS(x) x
#else /* Unix: no parameters in prototype! */
#define ARGS(x) ()
#endif
#endif

/* Include function prototypes for G722 operators and type definitions */
/* #include "operg722.h" */
#include "stl.h"

/* Define type for G.722 state structure */
typedef struct
{
  Word16          al[3];
  Word16          bl[7];
  Word16          detl;
  Word16          dlt[7]; /* dlt[0]=dlt */
  Word16          nbl;
  Word16          plt[3]; /* plt[0]=plt */
  Word16          rlt[3];
  Word16          ah[3];
  Word16          bh[7];
  Word16          deth;
  Word16          dh[7]; /* dh[0]=dh */
  Word16          ph[3]; /* ph[0]=ph */
  Word16          rh[3];
  Word16          sl;
  Word16          spl;
  Word16          szl;
  Word16          nbh;
  Word16          sh;
  Word16          sph;
  Word16          szh;
  Word16          qmf_tx_delayx[24];
  Word16          qmf_rx_delayx[24];
}          g722_state;

/* Include function prototypes for G722 functions */
#include "funcg722.h"

/* High-level (UGST) function prototypes for G722 functions */
void g722_reset_encoder ARGS((g722_state *encoder));
long g722_encode ARGS((short *incode, short *code, long nsmp, 
		       g722_state *encoder));
void g722_reset_decoder ARGS((g722_state *decoder));
short g722_decode ARGS((short *code, short *outcode, short mode, 
		       short nsmp, g722_state *decoder, struct WB_PLC_State *plc));
short g722_decode_plc ARGS((short *code, short *outcode, short mode, 
		       short nsmp, g722_state *decoder, struct WB_PLC_State *plc, short subf, short psml));

#endif /* G722_H */
/* ................. End of file g722.h .................................. */
