/* ITU G.722 3rd Edition (2012-09) */

/*--------------------------------------------------------------------------
 ITU-T Annex B (ex G.722-SWB) Source Code
 Software Release 1.00 (2010-09)
 (C) 2010 France Telecom, Huawei Technologies, NTT, VoiceAge Corp.
--------------------------------------------------------------------------*/

#include "dsputil.h"
#include "rom.h"

/*****************************/
#ifdef DYN_RAM_CNT
#include "dyn_ram_cnt.h"
#endif
/*****************************/

/*-------------------------------------------------------------------*
* Local function prototypes
*-------------------------------------------------------------------*/
static Word16 fcb_encode_pos(const Word16 pos_vector[], const Word16 pulse_num,
                             const Word16 pos_num);

/*-------------------------------------------------------------------*
* re8_compute_base_index:
*
* Compute base index for RE8
*-------------------------------------------------------------------*/
void re8_compute_base_index(
                            const Word16 *x,        /* i  : Elemen of Q2, Q3 or Q4                          */
                            const Word16 ka,        /* i  : Identifier of the absolute leader related to x  */
                            UWord16 *I         /* o  : index                                           */
                            )
{
  Word16 i, j, k1,m;
  Word16 setor_8p[8], setor_8p_temp[8];
  Word16 sign_8p;
  Word16 code_level, code_area;
  const Word16  *a1,*a2;
  Word16 code_index;
  UWord16 offset;

  /*****************************/
#ifdef DYN_RAM_CNT
  DYN_RAM_PUSH((UWord32) ((25 * SIZE_Word16) + (2 * SIZE_Ptr)), "dummy");
#endif
  /*****************************/

  a1 = Vals_a[ka];  move16();
  a2 = Vals_q[ka];  move16();

  /* the sign process */
  sign_8p = 0;      move16();
  m = 0;            move16();
  code_index = 0;   move16();
  k1 = a2[0];       move16();
  test();test();
  IF (sub(a2[1], 2) == 0 && s_xor(a1[0], 1) && sub(ka, 5))
  {
    FOR (i=0; i<8; i++)
    {
      IF (x[i] != 0)
      {
        sign_8p = shl(sign_8p, 1);
        setor_8p_temp[m++] = i;  move16();
      }
      if (x[i] < 0)
      {
        sign_8p = add(sign_8p, 1);
      }
    }

    code_index = fcb_encode_pos(setor_8p_temp,8,m);
    code_index = add(shl(code_index,k1),sign_8p);

    offset = IS_new[ka];                            move16();

    *I = extract_l(L_add(offset, code_index));
    move16();
  }
  ELSE
  {
    FOR (i=0;i<8;i++)
    {
      setor_8p[i] = abs_s(x[i]);		move16();	
      IF (x[i] != 0)
      {
        sign_8p = shl(sign_8p, 1);
        m = add(m,1);
      }
      if (x[i] < 0)
      {
        sign_8p = add(sign_8p, 1);
      }
    }

    if (sub(k1, m) != 0)
    {
      sign_8p = shr(sign_8p, 1);
    }

    /* code level by level */

    code_level = sub(a2[1], 1);
    code_area = 8;              move16();
    IF (sub(a2[2], 1) != 0)
    {
      FOR (j=0; j<code_level; j++)
      {
        m = 0;              move16();
        FOR (i = 0; i < code_area; i++)
        {
          IF (sub(setor_8p[i], a1[j]) != 0)
          {
            setor_8p_temp[m] = i;       move16();
            setor_8p[m] = setor_8p[i];  move16();
            m = add(m, 1);
          }
        }
        code_index = extract_l(L_mult0(code_index, Select_table22[m][code_area]));
        code_index = add(code_index, fcb_encode_pos(setor_8p_temp, code_area, m));
        code_area = m;                      move16();
      }
    }
    ELSE
    {
      FOR (i=0; i<code_area; i++)
      {
        if (sub(setor_8p[i], a1[1]) == 0)
        {
          code_index = add(code_index, i);
        }
      }
    }

    code_index = add(shl(code_index, k1), sign_8p);
    offset = IS_new[ka];                            move16();

    *I = extract_l(L_add(offset, code_index));
    move16();
  }

  /*****************************/
#ifdef DYN_RAM_CNT
  DYN_RAM_POP();
#endif
  /*****************************/ 

}

/*-------------------------------------------------------------------*
* fcb_encode_pos:
*
* Base function to compute base index for RE8
*-------------------------------------------------------------------*/
static Word16 fcb_encode_pos(    /* o  : Code index              */
                             const Word16 pos_vector[],      /* i  : Position vectort        */
                             const Word16 pulse_num,         /* i  : Pulse number            */
                             const Word16 pos_num            /* i  : Position number         */
                             )
{
  Word16 i, j;
  Word16 code_index;
  Word16 temp, temp1;
  Word16 Iters;

  const Word16 *select_table23;

  /*****************************/
#ifdef DYN_RAM_CNT
  DYN_RAM_PUSH((UWord32) ((6 * SIZE_Word16) + SIZE_Ptr), "dummy");
#endif
  /*****************************/

  temp = sub(pulse_num, 1);

  select_table23 = Select_table22[pos_num];  move16();

  code_index = sub(select_table23[pulse_num],select_table23[sub(pulse_num,pos_vector[0])]);

  j = 1;  move16();

  Iters = sub(pos_num, 1);
  FOR (i = 0; i < Iters; i++)
  {
    temp1 = sub(pos_num, j);

    select_table23 = Select_table22[temp1];  move16();

    code_index = add(code_index,sub( select_table23[sub(temp, pos_vector[i])],select_table23[sub(pulse_num,pos_vector[j])]) );

    j = add(j, 1);
  }

  /*****************************/
#ifdef DYN_RAM_CNT
  DYN_RAM_POP();
#endif
  /*****************************/ 

  return code_index;
}

