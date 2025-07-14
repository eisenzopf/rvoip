/* ITU G.722 3rd Edition (2012-09) */

/*--------------------------------------------------------------------------
 ITU-T G.722 Annex C (ex G.722-SWB-Float) Source Code
 Software Release 1.01 (2012-07)
 (C) 2012 France Telecom, Huawei Technologies, VoiceAge Corp., NTT.
--------------------------------------------------------------------------*/

#include "rom.h"


/*-------------------------------------------------------------------*
* Local function prototype
*-------------------------------------------------------------------*/
static void fcb_decode_pos_flt(Short index, Short pos_vector[], Short pulse_num, Short pos_num);



static Short search_ka_flt(unsigned short I, const unsigned short *ptr0, const Short *ptr1, Short nb) {
  Short i,j;

  i = 0;
  for(j=0; j<nb; j++)
  {
    if(I >= *(++ptr0))
    {
      i = i + 1;
    }
  }

  return(ptr1[i]);
}


/*-------------------------------------------------------------------*
* re8_decode_base_index
*
* Decode RE8 base index
*-------------------------------------------------------------------*/
void re8_decode_base_index_flt(
                           Short n,  /* i  : codebook number (*n is an integer defined in {0,2,3,4,..,n_max}) */
                           unsigned short I,  /* i  : index of c (pointer to unsigned 16-bit word)                     */
                           Short *x  /* o  : point in RE8 (8-dimensional integer vector)                      */
                           )
{
  Short i,j,k1,l,m,m1;
  Short setor_8p_temp[8],setor_8p_temp_1[8];
  Short setor_8p_temp_2 = 0; 
  Short sign_8p;
  Short code_level;
  const Short *a1,*a2;

  Short ka;
  Short offset;
  Short code_index;

  Short ii, iii;
  Short*	ptr;

  if (n < 2)
  {
    zeroS( 8, x);
  }
  else
  {
    if (I > 65519 )
    {
      I = 0;
    }

    /*-------------------------------------------------------------------*
    * search for the identifier ka of the absolute leader (table-lookup)
    * Q2 is a subset of Q3 - the two cases are considered in the same branch
    *-------------------------------------------------------------------*/
    if (n <= 3)
    {
      ka = search_ka_flt(I, I3_, A3_, NB_LDQ3);
    }
    else
    {
      ka = search_ka_flt(I, I4_, A4_, NB_LDQ4);
    }
    /*-------------------------------------------------------*
    * decode
    *-------------------------------------------------------*/
    a1 = Vals_a[ka];    
    a2 = Vals_q[ka];    
    k1 = a2[0];         
    code_level = a2[1]; 
    offset = IS_new[ka];
    code_index = (Short)(I - offset);

    sign_8p = code_index & ((1 << k1) - 1);

    code_index = code_index >> k1;

    for(i=0; i<8; i++)
    {
      x[i] = a1[0];
    }
    if(code_level > 1)
    {
      m = a2[2]; 
      m1 = a2[3];
      switch (code_level)
      {
        case 4:
          setor_8p_temp_2 = code_index & 1;
          code_index = code_index >> 1;

        case 3:
          l = Select_table22[m1][m];
          j = (code_index * DIV_mult[l]) >> DIV_shift[l];
          code_index = code_index - j * l;
          fcb_decode_pos_flt(code_index,setor_8p_temp_1, m, m1);
          code_index = j;
        case 2:
          fcb_decode_pos_flt(code_index,setor_8p_temp,8,m);
      }
      for (i=0;i<m;i++)
      {
        ii = setor_8p_temp[i];
        x[ii] = a1[1];
      }

      if(code_level > 2)
      {
        for (i=0;i<m1;i++)
        {
          ii = setor_8p_temp_1[i];
          iii = setor_8p_temp[ii];
          x[iii] = a1[2];
        }
        if(code_level > 3)
        {
          ii = setor_8p_temp_1[setor_8p_temp_2];
          iii = setor_8p_temp[ii];
          x[iii] = 6;
        }
      }
    }

    /*--------------------------------------------------------------------*
    * add the sign of all element ( except the last one in some case )
    *--------------------------------------------------------------------*/
    ptr = x+ 7;
    if((x[0] & 1) != 0)
    {
      /* odd covectors */
      m1 = *ptr--;
      /*--------------------------------------------------------------------*
      * add the sign of all elements  except the last one 
      *--------------------------------------------------------------------*/
      for (i=0; i<7; i++)
      {
        if ((sign_8p & 1) != 0)
        {
          *ptr = -*ptr;
        }
        sign_8p = sign_8p >> 1; 
        m1 = m1 + *ptr--;
      }
      /*--------------------------------------------------------------------*
      * recover the sign of last element if needed
      *--------------------------------------------------------------------*/
      if (m1 & 3)
      {
        x[7] = -x[7];
      }
    }
    else 
    {
      /* even covectors */
      /*--------------------------------------------------------------------*
      * add the sign of all non zero elements  
      *--------------------------------------------------------------------*/
      for (i=0; i<8; i++)
      {
        if (*ptr != 0)
        {
          if ((sign_8p & 1) != 0)
          {
            *ptr = -*ptr;
          }
          sign_8p = sign_8p >> 1; 
        }
        ptr--;
      }
    }
  }
}



/*-------------------------------------------------------------------*
* fcb_decode_pos
*
* base function for decoding position index
*-------------------------------------------------------------------*/
static void fcb_decode_pos_flt(
                           Short index,             /* i  : Index to decoder    */
                           Short pos_vector[],      /* o  : Position vector     */
                           Short pulse_num,         /* i  : Number of pulses    */
                           Short pos_num            /* i  : Number of positions */
                           )
{
  Short i,k,l;
  Short temp1,temp2;
  const Short *select_table23;
  const Short *select_table24;
  Short temp;

  k = index;        
  l = 0;            
  temp1 = pos_num;  
  temp2 = pulse_num + 1;
  temp = pos_num - 1;
  for (i=0;i<temp;i++)
  {
    select_table23 = Select_table22[temp1] ;           
    select_table24 = &select_table23[pulse_num - l];

    k = *select_table24 - k;                        
    while (k <= *select_table24)
    {
      l = l + 1;
      select_table24--;
    }

    k = select_table23[temp2 - l] - k;
    pos_vector[i] = l - 1;                          
    temp1 = temp1 - 1;
  }
  pos_vector[i] = l + k;
}
