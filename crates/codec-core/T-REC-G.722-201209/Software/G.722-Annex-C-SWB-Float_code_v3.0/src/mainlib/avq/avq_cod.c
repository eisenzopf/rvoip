/* ITU G.722 3rd Edition (2012-09) */

/*--------------------------------------------------------------------------
 ITU-T G.722 Annex C (ex G.722-SWB-Float) Source Code
 Software Release 1.01 (2012-07)
 (C) 2012 France Telecom, Huawei Technologies, VoiceAge Corp., NTT.
--------------------------------------------------------------------------*/

#include "re8.h"
#include "avq.h"
#include "bit_op.h"
#include "bwe.h"


#define FAC_LOG2   3.321928095f

/*-------------------------------------------------------------------*
* Function prototypes
*-------------------------------------------------------------------*/
static void RE8_Cod(
                    Short x[],  /* i  : point in RE8 (8-dimensional integer vector)                         */
                    Short *n,   /* i  : codebook number (*n is an integer defined in {0,2,3,4,..,n_max})    */
                    unsigned short *I, /* o  : index of c (pointer to unsigned 16-bit word)                        */
                    Short k[]   /* o  : index of v (8-dimensional vector of binary indices) = Voronoi index */
);


/*--------------------------------------------------------------*
* calc_bits(nq)
*
* COMPUTE (NUMBER OF BITS -1) TO DESCRIBE Q #nq
*--------------------------------------------------------------*/
static Short Calc_bits( /* o  : bit allocation            */
                        Short nq            /* i  : quantizer id (0,2,3,4...) */
                        )
{
  if (nq >= 2)
  {
    /*-----------------------------------------------------*
    * 4n bits + variable-length descriptor for allocation:
    *  descriptor -> nq
    *  0          -> 0
    *  10         -> 2
    *  110        -> 3
    *  => size of descriptor = 5n bits
    *-----------------------------------------------------*/
    return nq*5 - 1;
  }
  return 0; /* 1-1 [1 bit to describe the allocation] */
}



/*-----------------------------------------------------------------*
*   Function  AVQ_encmux_bstr                                     *
*            ~~~~~~~~~~~~~~~                                      *
*   Encode subvectors and write indexes into one bitstream.       *
*-----------------------------------------------------------------*/

Short AVQ_encmux_bstr(   /* o:   number of unused bits                        */
                       Short xriq[],        /* i/o: rounded subvectors [0..8*Nsv-1] followed
                                                    by rounded bit allocations [8*Nsv..8*Nsv+Nsv-1] */
                       unsigned short **pBst,      /* i/o: pointer to the bitstream buffer              */
                       const Short nb_bits, /* i:   number of allocated bits                     */
                       const Short Nsv      /* i:   number of subvectors                         */
)
{
  Short i, j, bits, pos, pos_max, overflow;
  Short sort_idx[NSV_MAX];
  Short nq[NSV_MAX], kv[NSV_MAX*8];
  unsigned short I[NSV_MAX];
  Short tmp16;
  Short *ptr;

  zeroS( NSV_MAX , nq );

  /*  encode subvectors and fix possible overflows in TOTAL bit budget:
  i.e. find for each subvector a codebook index nq (nq=0,2,3,4,...),
  a base codebook index (I), and a Voronoi index (kv)                         */
  /* ============================================================================ */

  /* sort subvectors by estimated bit allocations in decreasing order */
  /* use vector 'kv' as temporary to save memory */
  sort(&xriq[Nsv << 3], Nsv, sort_idx, kv);

  /* compute multi-rate indices and avoid bit budget overflow */
  pos_max = 0;
  bits = 0;
  for( i=0; i<Nsv; i++ )
  {
    /* find vector to quantize (criteria: nb of estimated bits) */
    pos = sort_idx[i];
    j = pos << 3;
    ptr = xriq + j;

    /* compute multi-rate index of rounded subvector (nq,I,kv[]) */
    RE8_Cod( ptr, &nq[pos], &I[pos], &kv[j] );

    if (nq[pos] > 0)
    {
      j = pos;
      if (j < pos_max)
          j = pos_max;

      /* check for overflow and compute number of bits-1 (n) */
      overflow = Calc_bits(nq[pos]);

      /* check for overflow and compute number of bits-1 (n) */
      if ((bits + overflow + j) <= nb_bits)
      {
        bits = bits + overflow;
        /* update index of the last described subvector */
        pos_max = j;
      }
      else
      {
        /* if budget overflow */
        zeroS(8, ptr);
        nq[pos] = 0;
      }
    }
  }

  /* write indexes to the bitstream */
  /* ============================== */
  bits = nb_bits;
  overflow = 0;
  for( i=0; i<Nsv; i++ )
  {
    /* 5*nq[i]-1 */
    j = Calc_bits(nq[i]);
    if (j == bits) /* check the overflow */
    {
      overflow = 1;
    }

    if( bits > 8 )
    {
      /* write the unary code for nq[i] */
      tmp16 = nq[i] - 1;

      for (j = tmp16; j > 0; j--)
      {
        *(*pBst)++ = ITU_G192_BIT_1;
      }
      /* we do not sub for tmp16 < 0 */
      if (tmp16 > 0)
      bits = bits - tmp16;

      if( !overflow )
      {
        /* write the stop bit */
        *(*pBst) = ITU_G192_BIT_0;
        (*pBst)++;								
        bits = bits - 1;
      }
      /* write codebook indices (rank I and event. Voronoi index kv) */
      if( nq[i] != 0 )    /* for Q0 nothing to write */
      {
        if( nq[i] < 5 )
        {
          tmp16 = nq[i] << 2;
          pos = 0;
        }
        else            /* for Q3/Q4 + Voronoi extensions r=1,2 */
        {
          j = 1;
          if ((nq[i] & 1) == 0 )    
          {
            j = j + 1;
          }
          pos = (nq[i] >> 1) - j;  /* Voronoi order determination */
          tmp16 = (j + 2) << 2;
        }

        s_PushBitLong(I[i], pBst, tmp16 );
        bits = bits - tmp16;

        if( pos > 0 )
        {
          tmp16 = i << 3;
          for( j=0; j<8; j++ )
          {
            /* kv[tmp16++] not counted because could be a pointer */
            s_PushBitLong( (long) kv[tmp16++], pBst, pos );
          }
          bits = bits - (pos << 3);
        }
      }
    }
  }

  /*
  * BIT PACKING/UNPACKING IS USUALLY NOT INSTRUMENTED
  */
  for( i=0; i < bits; i++ )   /* fill the rest of the bitstream */
  {
    *(*pBst) = ITU_G192_BIT_0;
    (*pBst)++;					
  }

  return (bits);
}



/*-------------------------------------------------------------------*
* AVQ_cod
*
* SPLIT ALGEBRAIC VECTOR QUANTIZER BASED ON RE8 LATTICE
* NOTE: a mitsmatch can occurs in some subvectors between the encoder
*       and decoder, because the encoder use a bit-rate estimator to set
*       the TCX global gain - this estimator is many times faster than the
*       call of RE8_idx() for bits calculation.
*-------------------------------------------------------------------*/
void AVQ_cod(
             Float *xri,    /* i  : vector to quantize                      */
             Short *xriq,   /* o  : quantized normalized vector (assuming the bit budget is enough) */
             Short NB_BITS, /* i  : number of allocated bits                */
             Short Nsv      /* i  : number of subvectors (lg=Nsv*8)         */
             )
{
  Short i, l, iter;

  Float gain_inv, nbits, nbits_max, fac, offset;

  Float ebits[NSV_MAX];
  Float ener, tmp;
  Float x1[8];

  Float *ptr;
  Short *ptrq;

  /* find energy of each subvector in log domain (scaled for bits estimation) */
  for( i=0; i<Nsv; i++ )
  {
      ener = 2.0f; /* to set ebits >= 0 */

      for( l=0; l<8; l++ )
      {
          tmp = xri[i*8+l];
          ener += tmp*tmp;
      }

      /* estimated bit consumption when gain=1 */ 
      ebits[i] = 5.0f * FAC_LOG2 * Log10( ener*0.5f );
  }

  /* estimate gain according to number of bits allowed */
  /* start at the middle (offset range = 0 to 255.75) */
  fac = 128.0f;      
  offset = 0.0f;
  nbits_max = 0.95f * ((float)(NB_BITS - Nsv));

  /* tree search with 10 iterations : offset with step of 0.25 bits (0.3 dB) */
  for( iter=0; iter<10; iter++ )
  {
      offset += fac;
      /* calculate the required number of bits */
      nbits = 0.0;
      for( i=0; i<Nsv; i++ )
      {
          tmp = ebits[i] - offset;
          if( tmp < 0.0 )
          {
              tmp = 0.0;
          }
          nbits += tmp;
      }
      /* decrease gain when no overflow occurs */
      if( nbits <= nbits_max )
      {
          offset -= fac;
      }
      fac *= 0.5;
  } 

  /* estimated gain (when offset=0, estimated gain=1) */
  gain_inv = 1.0f / Pow(10.0f, offset / (2.0f*5.0f*FAC_LOG2) );

  /* quantize all subvector using estimated gain */
  ptr = xri;
  ptrq = xriq;
  for (l=0; l<Nsv; l++)
  {
    for (i=0; i<8; i++)
    {
      x1[i] = *ptr++ * gain_inv;
    }
    RE8_ppv(x1, ptrq);
    ptrq+=8;
  }

  /* round bit allocations and save */
  for (i=0; i<Nsv; i++)
  {
    *ptrq++ = (Short)Floor(ebits[i] * 128.0f);
  }
}



/*------------------------------------------------------------------------
* RE8_cod:
*
* MULTI-RATE INDEXING OF A POINT y in THE LATTICE RE8 (INDEX COMPUTATION)
* note: the index I is defined as a 32-bit word, but only
*       16 bits are required (long can be replaced by unsigned integer)
*--------------------------------------------------------------------------*/
static void RE8_Cod(
                    Short x[],  /* i  : point in RE8 (8-dimensional integer vector)                         */
                    Short *n,   /* i  : codebook number (*n is an integer defined in {0,2,3,4,..,n_max})    */
                    unsigned short *I,  /* o  : index of c (pointer to unsigned 16-bit word)                        */
                    Short k[]   /* o  : index of v (8-dimensional vector of binary indices) = Voronoi index */
)
{
  Short ka, c[8];

  /*----------------------------------------------------------------------
  * decompose x as x = 2^r c + v, where r is an integer >=0, c is an element
  *  of Q0, Q2, Q3 or Q4, and v is an element of a Voronoi code in RE8
  *  (if r=0, x=c)
  *  this decomposition produces as a side-product the index k[] of v
  *  and the identifier ka of the absolute leader related to c
  *
  *  the index of y is split into 2 parts :
  *  - the index I of c
  *  - the index k[] of v
  ----------------------------------------------------------------------*/
  RE8_vor(x, n, k, c, &ka);
  /* compute the index I (only if c is in Q2, Q3 or Q4) */
  if (*n > 0)
  {
    s_re8_compute_base_index(c, ka, I);
  }
}


/* calculate subband energy */
void f_loadSubbandEnergy (Short cod_Mode, Float *fEnv_BWE, Float *fFenv_BWE , Short index_g_5bit)
{
  Float *ptr;
  int i;

  if(cod_Mode == TRANSIENT )
  {
    ptr = fFenv_BWE;
    for(i=0; i<N_SV_2; i++)
    {
      *ptr++= fEnv_BWE[i] / Pow( 2.0f , (Float)index_g_5bit );
      *ptr++ = fEnv_BWE[i]/ Pow( 2.0f , (Float)index_g_5bit );
    }
  }
  else
  {
	for(i=0; i<N_SV; i++)
		fFenv_BWE[i] = fEnv_BWE[i] / Pow( 2.0f , (Float)index_g_5bit );
  }
  return;
}
