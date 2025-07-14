/* ITU G.722 3rd Edition (2012-09) */

/*-----------------------------------------------------------------------------------
 ITU-T G.722-SWBS / G.722 Annex D - Reference C code for fixed-point implementation          
 Version 1.0
 Copyright (c) 2012,
 Huawei Technologies
-----------------------------------------------------------------------------------*/

/*--------------------------------------------------------------------------
 ITU-T Annex B (ex G.722-SWB) Source Code
 Software Release 1.00 (2010-09)
 (C) 2010 France Telecom, Huawei Technologies, NTT, VoiceAge Corp.
--------------------------------------------------------------------------*/

#include "bit_op.h"
#include "pcmswb_common.h"
#include "re8.h"
#include "avq.h"

#include "rom.h"

/*****************************/
#ifdef DYN_RAM_CNT
#include "dyn_ram_cnt.h"
#endif
/*****************************/

/*-------------------------------------------------------------------*
* Function prototypes
*-------------------------------------------------------------------*/

static void RE8_Dec(
                    Word16 n,     /* i  : codebook number (*n is an integer defined in {0,2,3,4,..,n_max})*/
                    UWord16 I,     /* i  : index of c (pointer to unsigned 16-bit word)            */
                    Word16 k[],   /* i  : index of v (8-dimensional vector of binary indices) = Voronoi index */
                    Word16 y[]    /* o  : point in RE8 (8-dimensional integer vector)             */
);

/*-----------------------------------------------------------------*
*   Function  AVQ_Demuxdec_Bstr                                   *
*            ~~~~~~~~~~~~~~~~~~                                   *
*   Read indexes from one bitstream and decode subvectors.        *
*-----------------------------------------------------------------*/

Word16 AVQ_Demuxdec_Bstr(
                         UWord16 *pBst,    /* i/o: pointer to the bitstream buffer */
                         Word16  xriq[],  /* o:   decoded subvectors [0..8*Nsv-1] */
                         const Word16  nb_bits, /* i:   number of allocated bits        */
                         const Word16  Nsv      /* i:   number of subvectors            */
                         )
{
    Word16 i;

    UWord16 Index;
    Word16 nq, kv[8];

    Word16 tmp16, bits, order_v, j;

    /*****************************/
#ifdef DYN_RAM_CNT
    DYN_RAM_PUSH((UWord32) (((7+ 8) * SIZE_Word16) + SIZE_Ptr), "dummy");
#endif
    /*****************************/

    bits = nb_bits;     move16();
    FOR( i=0; i<Nsv; i++ )
    {
        zero16_8(xriq);
        Index = 0; move16();
        nq = 0; move16();
        IF( sub(bits, 8) > 0 )
        {
            /* read the unary code including the stop bit for nq[i] */
            WHILE( sub(*pBst++, ITU_G192_BIT_1) == 0 )
            {
                nq= add(nq,1); 
                /* 5*nq[i]+4 == bits */
                tmp16 = sub(nq_table[nq], bits);

                IF(tmp16 == 0) /* check the overflow */
                {
                    bits = add(bits, 1);     /* overflow stop bit */
                    BREAK;
                }

            }
            bits = sub(bits, add(nq,1));
            if( nq > 0 )
            {
                nq = add(nq,1); 
            }

            /* read codebook indices (rank I and event. Voronoi index kv) */
            IF( nq != 0 )    /* for Q0 nothing to read */
            {
                IF( sub(nq,5) < 0 )    /* Q2, Q3, Q4 */
                {
                    tmp16 = shl(nq, 2);
                    order_v = 0;                        move16();
                }
                ELSE            /* for Q3/Q4 + Voronoi extensions r=1,2 */
                {
                    j= sub(2, s_and(nq,1));
                    order_v = sub(shr(nq, 1), j);  /* Voronoi order determination */
                    tmp16 = shl(add(2,j), 2);
                }
                Index = (Word16)GetBitLong( &pBst, tmp16 );      move16();
                bits = sub(bits, tmp16);
                IF( order_v > 0 )
                {
                    FOR( j=0; j<8; j++ )
                    {
                        kv[j] = (Word16)GetBitLong( &pBst, order_v ); move16();
                    }

                    bits = sub(bits, shl(order_v,3));
                } /* end if order_v > 0 */
            } /* end if nq[i] != 0 */
        } /* end if bits > 8*/
        IF (sub( nq, 2 ) >= 0)
        {
            RE8_Dec( nq, Index, kv, xriq );
        }
        xriq += 8;
    } /* loop i :0 -> Nsv */

        /*****************************/
#ifdef DYN_RAM_CNT
        DYN_RAM_POP();
#endif
        /*****************************/ 

    return bits;
}


/*--------------------------------------------------------------------------
* RE8_dec:
*
* MULTI-RATE INDEXING OF A POINT y in THE LATTICE RE8 (INDEX DECODING)
* note: the index I is defined as a 32-bit word, but only
* 16 bits are required (long can be replaced by unsigned integer)
*--------------------------------------------------------------------------*/
void RE8_Dec(
             Word16 n,     /* i  : codebook number (*n is an integer defined in {0,2,3,4,..,n_max})*/
             UWord16 I,     /* i  : index of c (pointer to unsigned 16-bit word)            */
             Word16 k[],   /* i  : index of v (8-dimensional vector of binary indices) = Voronoi index */
             Word16 y[]    /* o  : point in RE8 (8-dimensional integer vector)             */
)
{
  Word16 i, m, v[8];

  /*****************************/
#ifdef DYN_RAM_CNT
  DYN_RAM_PUSH((UWord32) (10 * SIZE_Word16), "dummy");
#endif
  /*****************************/

  /*------------------------------------------------------------------------*
  * decode the sub-indices I and kv[] according to the codebook number n:
  *  if n=0,2,3,4, decode I (no Voronoi extension)
  *  if n>4, Voronoi extension is used, decode I and kv[]
  *------------------------------------------------------------------------*/
  IF (sub(n, 4) <= 0)
  {
    re8_decode_base_index(n, I, y);
  }
  ELSE
  {
    /*--------------------------------------------------------------------*
    * compute the Voronoi modulo m = 2^r where r is extension order
    *--------------------------------------------------------------------*/
    m = 1;                               move16();
    n = sub(n, 2);
    WHILE (sub(n, 4) > 0)
    {
      m = add(m, 1);
      n = sub(n, 2);
    }

    /*--------------------------------------------------------------------*
    * decode base codebook index I into c (c is an element of Q3 or Q4)
    *  [here c is stored in y to save memory]
    *--------------------------------------------------------------------*/

    re8_decode_base_index(n, I, y);

    /*--------------------------------------------------------------------*
    * decode Voronoi index k[] into v
    *--------------------------------------------------------------------*/
    RE8_k2y(k, m, v);

    /*--------------------------------------------------------------------*
    * reconstruct y as y = m c + v (with m=2^r, r integer >=1)
    *--------------------------------------------------------------------*/
    FOR (i=0; i<8; i++)
    {
      /* y[i] = m*y[i] + v[i] */
      y[i] = add(shl(y[i], m), v[i]);  move16();
    }
  }

  /*****************************/
#ifdef DYN_RAM_CNT
  DYN_RAM_POP();
#endif
  /*****************************/ 

}
