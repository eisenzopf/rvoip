/* ITU G.722 3rd Edition (2012-09) */

/*--------------------------------------------------------------------------
 ITU-T G.722 Annex C (ex G.722-SWB-Float) Source Code
 Software Release 1.01 (2012-07)
 (C) 2012 France Telecom, Huawei Technologies, VoiceAge Corp., NTT.
--------------------------------------------------------------------------*/

#include "bit_op.h"
#include "pcmswb_common.h"
#include "re8.h"
#include "avq.h"


/*-------------------------------------------------------------------*
* Function prototypes
*-------------------------------------------------------------------*/
static void RE8_Dec(
                    Short n,     /* i  : codebook number (*n is an integer defined in {0,2,3,4,..,n_max})*/
                    unsigned short I,   /* i  : index of c (pointer to unsigned 16-bit word)            */
                    Short k[],   /* i  : index of v (8-dimensional vector of binary indices) = Voronoi index */
                    Short y[]    /* o  : point in RE8 (8-dimensional integer vector)             */
);


/*-----------------------------------------------------------------*
*   Function  AVQ_demuxdec_bstr                                   *
*            ~~~~~~~~~~~~~~~~~~                                   *
*   Read indexes from one bitstream and decode subvectors.        *
*-----------------------------------------------------------------*/

Short AVQ_demuxdec_bstr(
                         unsigned short *pBst, /* i/o: pointer to the bitstream buffer */
                         Short  xriq[],        /* o:   decoded subvectors [0..8*Nsv-1] */
                         const Short nb_bits,  /* i:   number of allocated bits        */
                         const Short Nsv       /* i:   number of subvectors            */
                         )
{
  Short i,j, bits, order_v;

  unsigned short I[NSV_MAX];
  Short nq[NSV_MAX], *kv, code[8];
  Short tmp16;

  zeroS( NSV_MAX, (Short*)I);
  zeroS( 8, code);

  kv = xriq; /* reuse vector to save memory */
  bits = nb_bits;

  for( i=0; i<Nsv; i++ )
  {
    nq[i] = 0; /* initialization and also forced if the budget is exceeded */

    if(bits > 8)
    {
      /* read the unary code including the stop bit for nq[i] */
      while(*pBst++ == ITU_G192_BIT_1)
      {
        nq[i] = nq[i] + 1;
        tmp16 = nq[i]*5+4;
        if (tmp16 == bits) /* check the overflow */
        {
          bits = bits + 1;     /* overflown stop bit */
          break;
        }
      }
      bits = bits - nq[i] - 1; /* count the stop bit */

      if( nq[i] > 0 )
      {
        nq[i] = nq[i] + 1;
      }

      /* read codebook indices (rank I and event. Voronoi index kv) */
      if( nq[i] != 0 )    /* for Q0 nothing to read */
      {
        if( nq[i] < 5)
        {
          tmp16 = nq[i] * 4;
          order_v = 0;
        }
        else            /* for Q3/Q4 + Voronoi extensions r=1,2 */
        {
          j = 1;
          if( (nq[i]%2) == 0 )    
          {
            j = j + 1;
          }
          order_v = (nq[i] >> 1) - j;  /* Voronoi order determination */
          tmp16 = (j + 2) << 2;
        }

        I[i] = (Short)s_GetBitLong( &pBst, tmp16 );
        bits = bits - tmp16;

        if( order_v > 0 )
        {
          tmp16 = i << 3;
          for( j=0; j<8; j++ )
          {
            kv[tmp16+j] = (Short)s_GetBitLong( &pBst, order_v );
          }
          bits = bits - (order_v << 3);
        }
      }
    }
  }

  pBst += bits;       /* skip the rest of the bitstream */

  /* decode all subvectors */
  for( i=0; i<Nsv; i++ )
  {
    /* multi-rate RE8 decoder */
    RE8_Dec( nq[i], I[i], kv, code );
    kv += 8;

    /* write decoded RE8 vector to decoded subvector #i */
    movSS( 8, code, xriq);
    xriq += 8;
  }

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
             Short n,     /* i  : codebook number (*n is an integer defined in {0,2,3,4,..,n_max})*/
             unsigned short I,   /* i  : index of c (pointer to unsigned 16-bit word)            */
             Short k[],   /* i  : index of v (8-dimensional vector of binary indices) = Voronoi index */
             Short y[]    /* o  : point in RE8 (8-dimensional integer vector)             */
)
{
  Short i, m, v[8];

  /*------------------------------------------------------------------------*
  * decode the sub-indices I and kv[] according to the codebook number n:
  *  if n=0,2,3,4, decode I (no Voronoi extension)
  *  if n>4, Voronoi extension is used, decode I and kv[]
  *------------------------------------------------------------------------*/
  if (n <= 4)
  {
    re8_decode_base_index_flt(n, I, y);
  }
  else
  {
    /*--------------------------------------------------------------------*
    * compute the Voronoi modulo m = 2^r where r is extension order
    *--------------------------------------------------------------------*/
    m = 0;

    while (n > 4)
    {
      m = m + 1;
      n = n - 2;
    }

    /*--------------------------------------------------------------------*
    * decode base codebook index I into c (c is an element of Q3 or Q4)
    *  [here c is stored in y to save memory]
    *--------------------------------------------------------------------*/

    re8_decode_base_index_flt(n, I, y);

    /*--------------------------------------------------------------------*
    * decode Voronoi index k[] into v
    *--------------------------------------------------------------------*/
    RE8_k2y_flt(k, m, v);

    /*--------------------------------------------------------------------*
    * reconstruct y as y = m c + v (with m=2^r, r integer >=1)
    *--------------------------------------------------------------------*/
    for (i=0; i<8; i++)
    {
      /* y[i] = m*y[i] + v[i] */
      y[i] = (y[i] << m) + v[i];
    }
  }
}

