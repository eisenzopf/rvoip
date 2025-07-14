/* ITU G.722 3rd Edition (2012-09) */

/*--------------------------------------------------------------------------
 ITU-T G.722 Annex C (ex G.722-SWB-Float) Source Code
 Software Release 1.01 (2012-07)
 (C) 2012 France Telecom, Huawei Technologies, VoiceAge Corp., NTT.
--------------------------------------------------------------------------*/

#include "rom.h"


/*-------------------------------------------------------------------*
* Local function prototypes
*-------------------------------------------------------------------*/
static Short s_fcb_encode_pos(const Short pos_vector[], const Short pulse_num,
                             const Short pos_num);

/*-------------------------------------------------------------------*
* re8_compute_base_index:
*
* Compute base index for RE8
*-------------------------------------------------------------------*/
void s_re8_compute_base_index(
                            const Short *x,        /* i  : Element										   */
                            const Short ka,        /* i  : Identifier of the absolute leader related to x  */
                            unsigned short *I      /* o  : index                                           */
                            )
{
  Short i, j, k1,m;
  Short setor_8p[8], setor_8p_temp[8];
  Short sign_8p;
  Short code_level, code_area;
  const Short  *a1,*a2;
  Short code_index;
  unsigned short offset;
  Short tmp;


  a1 = Vals_a[ka];
  a2 = Vals_q[ka];

  /* the sign process */
  sign_8p = 0;
  m = 0;
  code_index = 0;
  k1 = a2[0];
  if( (a2[1] - 2) == 0 && a1[0]^1 && ka - 5 )
  {
    for(i=0; i<8; i++)
    {
      if(x[i] != 0)
      {
        sign_8p = sign_8p << 1;
        setor_8p_temp[m++] = i;
      }
      if (x[i] < 0)
      {
        sign_8p += 1;
      }
    }

    code_index = s_fcb_encode_pos(setor_8p_temp,8,m);
    code_index = (code_index << k1) + sign_8p;

    offset = IS_new[ka];

    *I = (Short)(offset + code_index);
  }
  else
  {
    for (i=0;i<8;i++)
    {
	  if( x[i] < 0 )
		  tmp = -x[i];
	  else
		  tmp = x[i];
      setor_8p[i] = tmp;
      if(x[i] != 0)
      {
        sign_8p = sign_8p << 1;
        m += 1;
      }
      if (x[i] < 0)
      {
        sign_8p += 1;
      }
    }

    if( k1 != m )
    {
      sign_8p = sign_8p >> 1;
    }

    /* code level by level */

    code_level = a2[1] - 1;
    code_area = 8;
    if ( a2[2] != 1 )
    {
      for (j=0; j<code_level; j++)
      {
        m = 0;
        for (i = 0; i < code_area; i++)
        {
          if ( setor_8p[i] != a1[j] )
          {
            setor_8p_temp[m] = i;
            setor_8p[m] = setor_8p[i];
            m += 1;
          }
        }
        code_index = (Short)(code_index * Select_table22[m][code_area]);
        code_index = code_index + s_fcb_encode_pos(setor_8p_temp, code_area, m);
        code_area = m;
      }
    }
	else
    {
      for (i=0; i<code_area; i++)
      {
        if ( setor_8p[i] == a1[1] )
        {
          code_index += i;
        }
      }
    }

    code_index = (code_index << k1) + sign_8p;
    offset = IS_new[ka];

    *I = (Short)(offset + code_index);
  }
}




/*-------------------------------------------------------------------*
* fcb_encode_pos:
*
* Base function to compute base index for RE8
*-------------------------------------------------------------------*/
static Short s_fcb_encode_pos(    /* o  : Code index              */
                             const Short pos_vector[],      /* i  : Position vectort        */
                             const Short pulse_num,         /* i  : Pulse number            */
                             const Short pos_num            /* i  : Position number         */
                             )
{
  Short i, j;
  Short code_index;
  Short temp, temp1;
  Short Iters;

  const Short *select_table23;

  temp = pulse_num - 1;

  select_table23 = Select_table22[pos_num];

  code_index = select_table23[pulse_num] - select_table23[pulse_num - pos_vector[0]];

  j = 1;

  Iters = pos_num - 1;
  for(i = 0; i < Iters; i++)
  {
    temp1 = pos_num - j;

    select_table23 = Select_table22[temp1];

    code_index = code_index + ( select_table23[temp - pos_vector[i]] - select_table23[pulse_num - pos_vector[j]] );

    j++;
  }

  return code_index;
}
