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

void g722_reset_decoder(g722_state *decoder);
Word32 G722PLC_decode(short *code, short *outcode, short mode, Word32 read1,
                      g722_state *decoder,void *plc_state);
void G722PLC_qmf_updstat ARGS((short *outcode, short nsmp, g722_state *decoder,
           short *lb_signal, short *hb_signal, void *plc_state));

#endif /* G722_H */
/* ................. End of file g722.h .................................. */
